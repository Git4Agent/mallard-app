mod reconciliation_regressions {
    use super::*;

    #[test]
    fn memory_and_restarted_caches_validate_the_same_file_changes() {
        for mutation in [
            "append", "truncate", "rewrite", "replace", "delete", "move", "root",
        ] {
            let fixture = fixture();
            let memory = cache();
            let (_, _, thread) = ids();
            store(&memory, &fixture, stored(&thread, &[&fixture.rollout], 3));
            assert_eq!(
                served(&memory, &fixture, 3),
                1,
                "{mutation}: memory baseline"
            );
            assert_eq!(
                served(&cache(), &fixture, 3),
                1,
                "{mutation}: disk baseline"
            );
            let modified = std::fs::metadata(&fixture.rollout)
                .unwrap()
                .modified()
                .unwrap();
            let mut held_writer = None;
            match mutation {
                "append" => {
                    let mut writer = std::fs::OpenOptions::new()
                        .append(true)
                        .open(&fixture.rollout)
                        .unwrap();
                    writer.write_all(b"{\"a\":2}\n").unwrap();
                    writer.flush().unwrap();
                    held_writer = Some(writer);
                }
                "truncate" => std::fs::OpenOptions::new()
                    .write(true)
                    .open(&fixture.rollout)
                    .unwrap()
                    .set_len(1)
                    .unwrap(),
                "rewrite" => {
                    std::fs::write(&fixture.rollout, b"{\"a\":9}\n").unwrap();
                    #[cfg(unix)]
                    std::fs::OpenOptions::new()
                        .write(true)
                        .open(&fixture.rollout)
                        .unwrap()
                        .set_modified(modified)
                        .unwrap();
                }
                "replace" => {
                    let replacement = fixture.rollout.with_extension("replacement");
                    // Even identical bytes and mtime belong to a different file.
                    std::fs::write(&replacement, b"{\"a\":1}\n").unwrap();
                    std::fs::OpenOptions::new()
                        .write(true)
                        .open(&replacement)
                        .unwrap()
                        .set_modified(modified)
                        .unwrap();
                    std::fs::rename(replacement, &fixture.rollout).unwrap();
                }
                "delete" => std::fs::remove_file(&fixture.rollout).unwrap(),
                "move" => {
                    std::fs::rename(&fixture.rollout, fixture.home.join("moved.jsonl")).unwrap()
                }
                _ => {
                    let sessions = fixture.home.join("sessions");
                    std::fs::rename(&sessions, fixture.home.join("old-sessions")).unwrap();
                    std::fs::create_dir(&sessions).unwrap();
                    std::fs::write(&fixture.rollout, b"{\"a\":9}\n").unwrap();
                }
            }
            assert_eq!(
                served(&memory, &fixture, 3),
                0,
                "{mutation}: memory must reject"
            );
            assert_eq!(
                served(&cache(), &fixture, 3),
                0,
                "{mutation}: disk must reject"
            );
            drop(held_writer);
        }
    }

    #[test]
    fn child_and_ancestor_changes_invalidate_an_unchanged_parent_on_memory_and_disk() {
        for dependency in ["child", "ancestor"] {
            let fixture = fixture();
            let memory = cache();
            let (_, _, thread) = ids();
            let child = fixture.home.join("sessions/child.jsonl");
            let ancestor = fixture.home.join("sessions/ancestor.jsonl");
            for path in [&child, &ancestor] {
                std::fs::write(path, b"{\"message\":1}\n").unwrap();
            }
            let parent_stamp = SourceStamp::read(&fixture.rollout).unwrap();
            store(
                &memory,
                &fixture,
                stored(&thread, &[&fixture.rollout, &child, &ancestor], 3),
            );
            assert_eq!(served(&memory, &fixture, 3), 1);
            let changed = if dependency == "child" {
                &child
            } else {
                &ancestor
            };
            let mut writer = std::fs::OpenOptions::new()
                .append(true)
                .open(changed)
                .unwrap();
            writer.write_all(b"{\"message\":2}\n").unwrap();
            writer.flush().unwrap();
            assert_eq!(SourceStamp::read(&fixture.rollout).unwrap(), parent_stamp);
            assert_eq!(served(&memory, &fixture, 3), 0, "{dependency}: memory");
            assert_eq!(served(&cache(), &fixture, 3), 0, "{dependency}: disk");
            drop(writer);
        }
    }

    #[test]
    fn a_late_calculation_cannot_replace_a_newer_valid_result() {
        let fixture = fixture();
        let memory = cache();
        let (_, _, thread) = ids();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (finish_tx, finish_rx) = std::sync::mpsc::channel();
        std::thread::scope(|scope| {
            let fixture = &fixture;
            let memory = &memory;
            let thread = &thread;
            let old = scope.spawn(move || {
                let result = stored(&thread, &[&fixture.rollout], 3);
                started_tx.send(()).unwrap();
                finish_rx
                    .recv_timeout(std::time::Duration::from_secs(10))
                    .unwrap();
                store(&memory, &fixture, result);
            });
            started_rx
                .recv_timeout(std::time::Duration::from_secs(10))
                .unwrap();
            let mut writer = std::fs::OpenOptions::new()
                .append(true)
                .open(&fixture.rollout)
                .unwrap();
            writer.write_all(b"{\"a\":2}\n").unwrap();
            writer.flush().unwrap();
            store(&memory, &fixture, stored(&thread, &[&fixture.rollout], 4));
            assert_eq!(served(&memory, &fixture, 4), 1);
            finish_tx.send(()).unwrap();
            old.join().unwrap();
            for current in [&memory, &cache()] {
                assert_eq!(
                    served(current, &fixture, 3),
                    0,
                    "old result must remain rejected"
                );
                assert_eq!(
                    served(current, &fixture, 4),
                    1,
                    "new result must survive the late completion"
                );
            }
            drop(writer);
        });
    }

    #[test]
    fn refreshing_one_project_or_storage_does_not_validate_another_stale_record() {
        let fixture = fixture();
        let memory = cache();
        let (_, _, thread) = ids();
        let projects = ["project-a", "project-b"].map(|id| LocalProjectId::parse(id).unwrap());
        let storages = ["storage-a", "storage-b"].map(|id| StorageId::parse(id).unwrap());
        let count = |current: &ObservationCache, project: &LocalProjectId, storage: &StorageId| {
            current
                .take_reusable_observations(
                    &fixture.home,
                    &fixture.state_root,
                    project,
                    storage,
                    &query(&thread, 3),
                )
                .len()
        };
        for project in &projects {
            for storage in &storages {
                memory.store_observations(
                    &fixture.home,
                    &fixture.state_root,
                    project,
                    storage,
                    vec![stored(&thread, &[&fixture.rollout], 3)],
                );
                assert_eq!(count(&memory, project, storage), 1);
            }
        }
        let mut writer = std::fs::OpenOptions::new()
            .append(true)
            .open(&fixture.rollout)
            .unwrap();
        writer.write_all(b"{\"a\":2}\n").unwrap();
        writer.flush().unwrap();
        for project in &projects {
            for storage in &storages {
                assert_eq!(count(&memory, project, storage), 0);
            }
        }
        memory.store_observations(
            &fixture.home,
            &fixture.state_root,
            &projects[0],
            &storages[0],
            vec![stored(&thread, &[&fixture.rollout], 3)],
        );
        for current in [&memory, &cache()] {
            for (project_index, project) in projects.iter().enumerate() {
                for (storage_index, storage) in storages.iter().enumerate() {
                    let expected = usize::from(project_index == 0 && storage_index == 0);
                    assert_eq!(
                        count(current, project, storage),
                        expected,
                        "project {project_index}, storage {storage_index}"
                    );
                }
            }
        }
        drop(writer);
    }
}
