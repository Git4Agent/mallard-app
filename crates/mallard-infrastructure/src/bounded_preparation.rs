//! Bounded parallel preparation with serial consumption in completion order.
//!
//! Jobs start largest first on a fixed pool of workers. A job's bytes are
//! reserved only while it is being prepared: a job that does not fit beside
//! the running jobs waits at the head of the queue, and a job larger than the
//! whole budget runs alone. Each finished result is handed to `consume` on
//! the calling thread in completion order, tagged with the job's original
//! index. Completion order is not deterministic; callers that need a fixed
//! order sort by that index afterwards.

use std::collections::VecDeque;
use std::sync::{mpsc, Condvar, Mutex};

/// Bytes of transcript sources that may be in preparation at once across all
/// workers. A job reserves its declared size (transcript, sidecars, and the
/// declared Codex history base); its transient working set is a small
/// multiple of that.
pub const PREPARATION_BYTE_BUDGET: u64 = 256 * 1024 * 1024;

#[derive(Debug, Default)]
pub struct PreparationBounds {
    /// Most jobs in preparation at once.
    pub peak_jobs: usize,
    /// Most reserved bytes in preparation at once.
    pub peak_source_bytes: u64,
}

struct Queue<T> {
    waiting: VecDeque<(usize, u64, T)>,
    running_jobs: usize,
    running_bytes: u64,
    bounds: PreparationBounds,
}

/// Prepare every job on `workers` threads and hand each result to `consume`
/// as it completes. Reservations cover running jobs only: a job starts when
/// a worker is free and its declared size fits beside the jobs running now,
/// a job above the whole budget runs alone, and a job that does not fit
/// waits at the head of the queue with the idle workers. A panic inside
/// `prepare` is that job's own error; every other job still returns.
pub fn for_each_prepared<T: Send, R: Send>(
    jobs: Vec<T>,
    workers: usize,
    byte_budget: u64,
    size: impl Fn(&T) -> u64,
    prepare: impl Fn(T) -> Result<R, String> + Sync,
    mut consume: impl FnMut(usize, Result<R, String>),
) -> PreparationBounds {
    if jobs.is_empty() {
        return PreparationBounds::default();
    }
    let workers = workers.max(1).min(jobs.len());
    let count = jobs.len();
    let mut waiting: Vec<(usize, u64, T)> = jobs
        .into_iter()
        .enumerate()
        .map(|(index, job)| (index, size(&job), job))
        .collect();
    // Largest first; equal sizes keep their index order.
    waiting.sort_by(|left, right| right.1.cmp(&left.1).then(left.0.cmp(&right.0)));
    let queue = Mutex::new(Queue {
        waiting: waiting.into(),
        running_jobs: 0,
        running_bytes: 0,
        bounds: PreparationBounds::default(),
    });
    let freed = Condvar::new();
    std::thread::scope(|scope| {
        // Finished results wait here for the consumer; a worker whose send
        // blocks holds one more, so at most 2 × workers results are pending
        // and none of them holds a reservation.
        let (results, completed) = mpsc::sync_channel(workers);
        for _ in 0..workers {
            let results = results.clone();
            let (queue, freed, prepare) = (&queue, &freed, &prepare);
            scope.spawn(move || loop {
                let (index, size, job) = {
                    let mut guard = queue.lock().unwrap_or_else(|e| e.into_inner());
                    loop {
                        let Some(&(_, size, _)) = guard.waiting.front() else {
                            return;
                        };
                        // The head is admitted while nothing runs, so an
                        // oversized job runs alone instead of waiting forever.
                        if guard.running_jobs == 0
                            || guard.running_bytes.saturating_add(size) <= byte_budget
                        {
                            break;
                        }
                        guard = freed.wait(guard).unwrap_or_else(|e| e.into_inner());
                    }
                    let (index, size, job) = guard.waiting.pop_front().expect("head was checked");
                    guard.running_jobs += 1;
                    guard.running_bytes = guard.running_bytes.saturating_add(size);
                    guard.bounds.peak_jobs = guard.bounds.peak_jobs.max(guard.running_jobs);
                    guard.bounds.peak_source_bytes =
                        guard.bounds.peak_source_bytes.max(guard.running_bytes);
                    (index, size, job)
                };
                let outcome =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| prepare(job)))
                        .unwrap_or_else(|_| Err("thread preparation worker panicked".to_string()));
                {
                    let mut guard = queue.lock().unwrap_or_else(|e| e.into_inner());
                    guard.running_jobs -= 1;
                    guard.running_bytes = guard.running_bytes.saturating_sub(size);
                }
                freed.notify_all();
                if results.send((index, outcome)).is_err() {
                    return;
                }
            });
        }
        drop(results);
        for _ in 0..count {
            let (index, result) = completed
                .recv()
                .expect("every prepared job returns one result");
            consume(index, result);
        }
    });
    queue.into_inner().unwrap_or_else(|e| e.into_inner()).bounds
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::time::Duration;

    /// Gates between jobs and the consumer: a wait that never comes fails the
    /// job instead of hanging the test.
    const GATE_TIMEOUT: Duration = Duration::from_secs(5);

    #[test]
    fn results_match_serial_after_sorting_by_index_and_isolate_failed_preparation() {
        let run = |workers| {
            let mut outcomes = Vec::new();
            for_each_prepared(
                (0..8).collect(),
                workers,
                64,
                |_| 1,
                |job| {
                    if job == 3 {
                        Err("changed source".to_string())
                    } else {
                        Ok(job * 2)
                    }
                },
                |index, outcome| outcomes.push((index, outcome)),
            );
            outcomes.sort_by_key(|(index, _)| *index);
            outcomes
        };
        let serial = run(1);
        assert_eq!(serial, run(4));
        assert_eq!(
            serial.iter().filter(|(_, result)| result.is_ok()).count(),
            7
        );
        assert!(serial[3].1.is_err());
        assert_eq!(serial[7], (7, Ok(14)));
    }

    #[test]
    fn largest_jobs_start_first() {
        let sizes = [1, 9, 3];
        let started = Mutex::new(Vec::new());
        for_each_prepared(
            (0..sizes.len()).collect(),
            1,
            64,
            |&job| sizes[job],
            |job| {
                started.lock().unwrap().push(job);
                Ok(job)
            },
            |index, result| assert_eq!(result.unwrap(), index),
        );
        assert_eq!(started.into_inner().unwrap(), vec![1, 2, 0]);
    }

    #[test]
    fn finished_results_free_their_slot_and_bytes() {
        // Job 0 is the largest and runs until job 2 has started; job 1
        // finishes at once. Job 2 can only start on the slot and bytes job 1
        // freed while job 0 still runs. A reservation held until in-order
        // consumption would never release them here (job 0 is consumed
        // first), so this schedule pins running-only reservations.
        let (started_two, wait_for_two) = mpsc::channel();
        let wait_for_two = Mutex::new(wait_for_two);
        let sizes = [3, 1, 1];
        let mut consumed = Vec::new();
        let bounds = for_each_prepared(
            (0..sizes.len()).collect(),
            2,
            6,
            |&job| sizes[job],
            |job| {
                match job {
                    0 => wait_for_two
                        .lock()
                        .unwrap()
                        .recv_timeout(GATE_TIMEOUT)
                        .expect("job 2 starts while job 0 runs"),
                    2 => started_two.send(()).unwrap(),
                    _ => {}
                }
                Ok(job)
            },
            |index, result| consumed.push((index, result.expect("prepared"))),
        );
        consumed.sort();
        assert_eq!(consumed, vec![(0, 0), (1, 1), (2, 2)]);
        assert_eq!(bounds.peak_jobs, 2);
        assert_eq!(bounds.peak_source_bytes, 4);
    }

    #[test]
    fn results_are_consumed_in_completion_order() {
        // Job 0 waits on a gate the consumer opens after consuming job 1, so
        // the consumer must see index 1 first. In-order consumption would
        // deadlock here.
        let (open, gate) = mpsc::channel();
        let gate = Mutex::new(gate);
        let mut consumed = Vec::new();
        for_each_prepared(
            vec![0, 1],
            2,
            64,
            |_| 1,
            |job| {
                if job == 0 {
                    gate.lock()
                        .unwrap()
                        .recv_timeout(GATE_TIMEOUT)
                        .expect("job 1 is consumed while job 0 runs");
                }
                Ok(job)
            },
            |index, result| {
                assert_eq!(result.expect("prepared"), index);
                if index == 1 {
                    open.send(()).unwrap();
                }
                consumed.push(index);
            },
        );
        assert_eq!(consumed, vec![1, 0]);
    }

    #[test]
    fn oversized_job_runs_alone_and_queue_respects_worker_cap() {
        // The 9 is above the budget of 6: it starts first and nothing runs
        // beside it. Afterwards the 3s run in pairs on the two workers.
        let sizes = [3, 3, 9, 3, 3, 3];
        let started = Mutex::new(Vec::new());
        let oversized_running = AtomicBool::new(false);
        let small_started = AtomicUsize::new(0);
        let (paired, wait_for_pair) = mpsc::channel();
        let wait_for_pair = Mutex::new(wait_for_pair);
        let bounds = for_each_prepared(
            (0..sizes.len()).collect(),
            2,
            6,
            |&job| sizes[job],
            |job| {
                started.lock().unwrap().push(job);
                if job == 2 {
                    assert_eq!(*started.lock().unwrap(), vec![2], "largest first");
                    oversized_running.store(true, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(20));
                    assert_eq!(started.lock().unwrap().len(), 1, "runs alone");
                    oversized_running.store(false, Ordering::SeqCst);
                    return Ok(job);
                }
                assert!(!oversized_running.load(Ordering::SeqCst));
                // The first small job waits for a second one beside it.
                if small_started.fetch_add(1, Ordering::SeqCst) == 0 {
                    wait_for_pair
                        .lock()
                        .unwrap()
                        .recv_timeout(GATE_TIMEOUT)
                        .expect("a second small job runs beside the first");
                } else {
                    paired.send(()).unwrap();
                }
                assert!(!oversized_running.load(Ordering::SeqCst));
                Ok(job)
            },
            |index, result| assert_eq!(result.expect("prepared"), index),
        );
        let mut observed = started.into_inner().unwrap();
        assert_eq!(observed[0], 2);
        observed.sort();
        assert_eq!(observed, (0..sizes.len()).collect::<Vec<_>>());
        assert_eq!(bounds.peak_jobs, 2);
        assert_eq!(bounds.peak_source_bytes, 9);
    }

    #[test]
    fn a_job_that_does_not_fit_waits_at_the_head() {
        // Budget 10, three workers, sizes [8, 4, 1]: while 8 runs nothing
        // else starts (4 does not fit, and 1 waits behind it in queue order
        // although it would fit); after 8 finishes, 4 and 1 run together.
        let sizes = [8, 4, 1];
        let started = Mutex::new(Vec::new());
        let (one_started, wait_for_one) = mpsc::channel();
        let wait_for_one = Mutex::new(wait_for_one);
        let bounds = for_each_prepared(
            (0..sizes.len()).collect(),
            3,
            10,
            |&job| sizes[job],
            |job| {
                started.lock().unwrap().push(job);
                match job {
                    0 => {
                        std::thread::sleep(Duration::from_millis(20));
                        assert_eq!(*started.lock().unwrap(), vec![0], "the head waits");
                    }
                    1 => wait_for_one
                        .lock()
                        .unwrap()
                        .recv_timeout(GATE_TIMEOUT)
                        .expect("job 2 runs beside job 1"),
                    _ => one_started.send(()).unwrap(),
                }
                Ok(job)
            },
            |index, result| assert_eq!(result.expect("prepared"), index),
        );
        // Jobs 1 and 2 are admitted together; which one records its start
        // first is the workers' race, not the queue's order.
        let mut started = started.into_inner().unwrap();
        assert_eq!(started[0], 0);
        started[1..].sort();
        assert_eq!(started, vec![0, 1, 2]);
        assert_eq!(bounds.peak_jobs, 2);
        assert_eq!(bounds.peak_source_bytes, 8);
    }

    #[test]
    fn worker_panic_does_not_drop_sibling_results() {
        let mut outcomes = Vec::new();
        for_each_prepared(
            vec![0, 1, 2],
            2,
            6,
            |_| 1,
            |job| {
                assert_ne!(job, 1, "injected panic");
                Ok(job)
            },
            |index, result| outcomes.push((index, result)),
        );
        outcomes.sort_by_key(|(index, _)| *index);
        assert_eq!(outcomes[0], (0, Ok(0)));
        assert!(outcomes[1]
            .1
            .as_ref()
            .unwrap_err()
            .contains("worker panicked"));
        assert_eq!(outcomes[2], (2, Ok(2)));
    }
}
