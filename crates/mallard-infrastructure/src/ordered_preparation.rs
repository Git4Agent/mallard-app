//! Bounded preparation with serial, ordered consumption of each result.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{mpsc, Arc, Mutex};

#[derive(Debug, Default)]
pub struct PreparationBounds {
    pub peak_jobs: usize,
    pub peak_source_bytes: u64,
}

/// Reservations cover running jobs and completed results until consumed.
/// A job above the byte budget runs alone; its existing file limit still applies.
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
    let mut waiting: VecDeque<_> = jobs
        .into_iter()
        .enumerate()
        .map(|(index, job)| (index, size(&job), job))
        .collect();
    let mut bounds = PreparationBounds::default();
    std::thread::scope(|scope| {
        let (tasks, receiver) = mpsc::sync_channel::<(usize, T)>(workers);
        let receiver = Arc::new(Mutex::new(receiver));
        let (results, completed) = mpsc::sync_channel(workers);
        for _ in 0..workers {
            let receiver = receiver.clone();
            let results = results.clone();
            let prepare = &prepare;
            scope.spawn(move || loop {
                let task = receiver.lock().unwrap_or_else(|e| e.into_inner()).recv();
                let Ok((index, job)) = task else {
                    break;
                };
                let outcome =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| prepare(job)))
                        .unwrap_or_else(|_| Err("thread preparation worker panicked".to_string()));
                if results.send((index, outcome)).is_err() {
                    break;
                }
            });
        }
        drop(results);
        let mut admitted = BTreeMap::new();
        let mut source_bytes = 0_u64;
        let mut ready = BTreeMap::new();
        for next in 0..count {
            while admitted.len() < workers {
                let Some((_, size, _)) = waiting.front() else {
                    break;
                };
                if !admitted.is_empty() && source_bytes.saturating_add(*size) > byte_budget {
                    break;
                }
                let (index, size, job) = waiting.pop_front().unwrap();
                source_bytes = source_bytes.saturating_add(size);
                admitted.insert(index, size);
                bounds.peak_jobs = bounds.peak_jobs.max(admitted.len());
                bounds.peak_source_bytes = bounds.peak_source_bytes.max(source_bytes);
                tasks
                    .send((index, job))
                    .expect("preparation workers remain available");
            }
            while !ready.contains_key(&next) {
                let (index, result) = completed.recv().expect("admitted job returns one result");
                ready.insert(index, result);
            }
            consume(next, ready.remove(&next).unwrap());
            source_bytes -= admitted.remove(&next).unwrap();
        }
        drop(tasks);
    });
    bounds
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    #[test]
    fn consumes_in_order_and_isolates_failed_preparation() {
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
    fn completed_results_keep_their_byte_and_job_reservations() {
        let (finished, finish) = mpsc::channel();
        let finish = Mutex::new(finish);
        let started = AtomicUsize::new(0);
        let mut consumed = Vec::new();
        let bounds = for_each_prepared(
            (0..6).collect(),
            4,
            6,
            |_| 3,
            |job| {
                started.fetch_add(1, Ordering::SeqCst);
                if job == 0 {
                    finish
                        .lock()
                        .unwrap()
                        .recv_timeout(Duration::from_secs(5))
                        .unwrap();
                } else if job == 1 {
                    finished.send(()).unwrap();
                }
                Ok(job)
            },
            |index, result| {
                if index == 0 {
                    // Job 1 can finish before job 0. No third job may start
                    // until consumption releases one of their reservations.
                    assert_eq!(started.load(Ordering::SeqCst), 2);
                }
                consumed.push(result.unwrap());
            },
        );
        assert_eq!(consumed, (0..6).collect::<Vec<_>>());
        assert_eq!(bounds.peak_jobs, 2);
        assert_eq!(bounds.peak_source_bytes, 6);
    }

    #[test]
    fn oversized_job_runs_alone_and_queue_respects_worker_cap() {
        let started = Mutex::new(Vec::new());
        let sizes = [3, 3, 9, 3, 3, 3];
        let bounds = for_each_prepared(
            (0..sizes.len()).collect(),
            2,
            6,
            |&job| sizes[job],
            |job| {
                started.lock().unwrap().push(job);
                Ok(job)
            },
            |index, result| {
                assert_eq!(result.unwrap(), index);
                if index == 2 {
                    let mut observed = started.lock().unwrap().clone();
                    observed.sort();
                    assert_eq!(observed, vec![0, 1, 2]);
                }
            },
        );
        assert_eq!(bounds.peak_jobs, 2);
        assert_eq!(bounds.peak_source_bytes, 9);
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
            |_, result| outcomes.push(result),
        );
        assert_eq!(outcomes[0], Ok(0));
        assert!(outcomes[1]
            .as_ref()
            .unwrap_err()
            .contains("worker panicked"));
        assert_eq!(outcomes[2], Ok(2));
    }
}
