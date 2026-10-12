//! Move an existing chat through Desktop's authoritative sidebar operation.
//! Backend thread metadata alone does not update Desktop project membership.

#[cfg(target_os = "macos")]
pub(super) mod macos;
#[cfg(windows)]
mod windows;

use serde::Deserialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[cfg(windows)]
pub(super) fn assign_and_open(
    executable: &Path,
    home: &Path,
    thread_id: &str,
    project_root: &Path,
    register_project: impl FnMut() -> Result<(), String>,
) -> Result<(), String> {
    // Includes a cold Desktop start and its asynchronous folder registration.
    let deadline = Instant::now() + Duration::from_secs(60);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| "Could not start the Codex Desktop connection.".to_string())?;
    let mut desktop = runtime.block_on(windows::connect(executable, home, deadline))?;
    assign_with_request(
        thread_id,
        project_root,
        deadline,
        |tool, args, timeout| runtime.block_on(desktop.call(thread_id, tool, args, timeout)),
        register_project,
    )
    .map_err(|error| format!("Could not open the Codex chat in its project: {error}"))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProjectList {
    projects: Vec<Project>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Project {
    project_id: String,
    project_kind: String,
    host_id: Option<String>,
    path: Option<PathBuf>,
}

fn project_id_in_listing(value: Value, project_root: &Path) -> Result<Option<String>, String> {
    let listing: ProjectList = serde_json::from_value(value)
        .map_err(|_| "Codex Desktop returned an invalid project list.")?;
    if listing.projects.len() > 10_000 {
        return Err("Codex Desktop returned too many projects.".into());
    }
    let mut matches = listing.projects.iter().filter(|project| {
        project.project_kind == "local"
            && project.host_id.as_deref() == Some("local")
            && project
                .path
                .as_ref()
                .is_some_and(|path| crate::local_paths::same_canonical_path(path, project_root))
    });
    let Some(project) = matches.next() else {
        return Ok(None);
    };
    if matches.next().is_some() {
        return Err("More than one Codex Desktop project uses this folder.".into());
    }
    if project.project_id.trim().is_empty() || project.project_id.len() > 256 {
        return Err("Codex Desktop returned an invalid project ID.".into());
    }
    Ok(Some(project.project_id.clone()))
}

fn assign_with_request(
    thread_id: &str,
    project_root: &Path,
    deadline: Instant,
    mut request: impl FnMut(&str, Value, Duration) -> Result<Value, String>,
    #[cfg(windows)] mut register_project: impl FnMut() -> Result<(), String>,
) -> Result<(), String> {
    super::actions::validate_thread_uuid(thread_id)?;
    if !project_root.is_absolute() {
        return Err("Project root must be absolute.".into());
    }
    let remaining = || {
        deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(|| "Codex Desktop did not confirm project placement in time.".to_string())
    };
    #[cfg(windows)]
    let mut registration_requested = false;
    let project_id = loop {
        if let Some(project_id) = project_id_in_listing(
            request("list_projects", json!({}), remaining()?)?,
            project_root,
        )? {
            break project_id;
        }
        #[cfg(windows)]
        {
            // The profile-verified list request precedes any folder launch.
            // Send the link once, then wait for Desktop's authoritative ID.
            if !registration_requested {
                remaining()?;
                register_project()?;
                registration_requested = true;
            }
            std::thread::sleep(remaining()?.min(Duration::from_millis(250)));
        }
        #[cfg(not(windows))]
        std::thread::sleep(Duration::from_millis(100).min(remaining()?));
    };

    #[cfg(windows)]
    {
        // A restored chat can exist on disk before Desktop knows its source
        // project. Open it and confirm the exact local chat before moving it.
        // `notLoaded` describes the agent runtime. Stored history can be ready
        // before that runtime is loaded, but navigation alone is asynchronous.
        let opened = request(
            "navigate_to_codex_page",
            json!({"threadId": thread_id}),
            remaining()?,
        )?;
        if opened["navigated"] != true {
            return Err("Codex Desktop did not open the chat for project placement.".into());
        }
        loop {
            let status = request(
                "read_thread",
                json!({"threadId": thread_id, "hostId": "local", "turnLimit": 1,
                    "includeOutputs": false, "maxOutputCharsPerItem": 0}),
                remaining()?,
            )?;
            let thread = &status["thread"];
            if thread["id"] != thread_id || thread["kind"] != "codex" || thread["hostId"] != "local"
            {
                return Err("Codex Desktop did not confirm the requested local chat.".into());
            }
            match thread["status"]["type"].as_str() {
                Some("idle" | "active") => break,
                Some("notLoaded") => {
                    let turns = status["turns"]
                        .as_array()
                        .ok_or("Codex Desktop returned invalid chat history.")?;
                    if !turns.is_empty() {
                        break;
                    }
                    std::thread::sleep(remaining()?.min(Duration::from_millis(250)));
                }
                _ => {
                    return Err(
                        "Codex Desktop could not read the chat for project placement.".into(),
                    )
                }
            }
        }
    }

    // Send the move exactly once. Its reply confirms the native placement,
    // including for old chats outside list_threads' 50-item recency window.
    let moved = request(
        "move_thread_to_project",
        json!({"source": "codex", "hostId": "local", "threadId": thread_id,
            "projectId": project_id}),
        remaining()?,
    )?;
    if moved["source"] != "codex"
        || moved["hostId"] != "local"
        || moved["threadId"] != thread_id
        || moved["projectId"] != project_id
    {
        return Err("Codex Desktop did not confirm the chat's project placement.".into());
    }
    let opened = request(
        "navigate_to_codex_page",
        json!({"threadId": thread_id}),
        remaining()?,
    )?;
    if opened["navigated"] != true {
        return Err(
            "The chat was placed in its project, but Codex Desktop did not open it.".into(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const THREAD: &str = "019f7798-5437-7632-9dbc-5b589cf68bf0";
    const PROJECT: &str = "019f7798-5437-7632-9dbc-5b589cf68bf1";

    fn project(root: &Path) -> Value {
        json!({"projectId": PROJECT, "projectKind": "local", "hostId": "local", "path": root})
    }

    #[test]
    fn loads_and_moves_the_same_old_chat_in_the_exact_local_project() {
        let temp = tempfile::tempdir().unwrap();
        let canonical = std::fs::canonicalize(temp.path()).unwrap();
        let ordinary = dunce::simplified(&canonical);
        let mut calls = Vec::new();
        #[cfg(windows)]
        let mut reads = 0;
        assign_with_request(THREAD, &canonical, Instant::now() + Duration::from_secs(5), |tool, args, _| {
            calls.push(tool.to_string());
            match tool {
                "list_projects" => Ok(json!({"projects": [project(&ordinary.join("child")),
                    {"projectId":"remote", "projectKind":"remote", "hostId":"remote", "path":ordinary},
                    project(ordinary)]})),
                "move_thread_to_project" => {
                    #[cfg(windows)]
                    assert_eq!(reads, 1, "confirm the chat without waiting for its agent runtime");
                    assert_eq!(args, json!({"source":"codex", "hostId":"local", "threadId":THREAD, "projectId":PROJECT}));
                    Ok(args)
                }
                #[cfg(windows)]
                "read_thread" => {
                    assert_eq!(args["threadId"], THREAD);
                    assert_eq!(args["hostId"], "local");
                    reads += 1;
                    Ok(json!({"thread":{"id":THREAD, "kind":"codex", "hostId":"local",
                        "status":{"type":"notLoaded"}}, "turns":[{"id":"stored-turn"}]}))
                }
                "navigate_to_codex_page" => {
                    assert_eq!(args, json!({"threadId":THREAD}));
                    Ok(json!({"navigated":true}))
                }
                _ => panic!("unexpected native call"),
            }
        }, #[cfg(windows)] || panic!("existing projects must not be registered again")).unwrap();
        assert_eq!(
            calls,
            [
                "list_projects",
                #[cfg(windows)]
                "navigate_to_codex_page",
                #[cfg(windows)]
                "read_thread",
                "move_thread_to_project",
                "navigate_to_codex_page"
            ]
        );
    }

    #[test]
    fn waits_for_a_missing_folder_to_be_registered_before_moving_the_same_chat() {
        let temp = tempfile::tempdir().unwrap();
        let mut calls = Vec::new();
        let mut reads = 0;
        assign_with_request(
            THREAD,
            temp.path(),
            Instant::now() + Duration::from_secs(5),
            |tool, args, _| {
                calls.push(tool.to_string());
                match tool {
                    "list_projects" => {
                        reads += 1;
                        if reads == 1 {
                            Ok(json!({"projects":[]}))
                        } else if reads < 4 {
                            // A route acknowledgment is not proof of registration.
                            Ok(json!({"projects":[project(&temp.path().join("other"))]}))
                        } else {
                            Ok(json!({"projects":[project(temp.path())]}))
                        }
                    }
                    "move_thread_to_project" => {
                        assert_eq!(reads, 4);
                        assert_eq!(
                            args,
                            json!({"source":"codex", "hostId":"local",
                            "threadId":THREAD, "projectId":PROJECT})
                        );
                        Ok(args)
                    }
                    "navigate_to_codex_page" => {
                        assert_eq!(args, json!({"threadId":THREAD}));
                        Ok(json!({"navigated":true}))
                    }
                    #[cfg(windows)]
                    "read_thread" => Ok(json!({"thread":{"id":THREAD, "kind":"codex", "hostId":"local", "status":{"type":"idle"}}})),
                    _ => panic!("unexpected native call"),
                }
            },
            #[cfg(windows)]
            || Ok(()),
        )
        .unwrap();
        assert_eq!(
            calls,
            [
                "list_projects",
                "list_projects",
                "list_projects",
                "list_projects",
                #[cfg(windows)]
                "navigate_to_codex_page",
                #[cfg(windows)]
                "read_thread",
                "move_thread_to_project",
                "navigate_to_codex_page"
            ]
        );
    }

    #[test]
    fn unconfirmed_registration_times_out_without_moving_a_chat() {
        let temp = tempfile::tempdir().unwrap();
        let error = assign_with_request(
            THREAD,
            temp.path(),
            Instant::now() + Duration::from_millis(150),
            |tool, _, _| {
                assert_eq!(tool, "list_projects");
                Ok(json!({"projects":[]}))
            },
            #[cfg(windows)]
            || Ok(()),
        )
        .unwrap_err();
        assert!(error.contains("in time"), "{error}");
    }

    #[test]
    fn ambiguous_and_invalid_projects_never_register_or_move_a_chat() {
        let temp = tempfile::tempdir().unwrap();
        let mut invalid = project(temp.path());
        invalid["projectId"] = json!("");
        for projects in [
            json!([project(temp.path()), project(temp.path())]),
            json!([invalid]),
        ] {
            let error = assign_with_request(
                THREAD,
                temp.path(),
                Instant::now() + Duration::from_secs(5),
                |tool, _, _| {
                    assert_eq!(tool, "list_projects");
                    Ok(json!({"projects":projects}))
                },
                #[cfg(windows)]
                || panic!("invalid projects must not trigger registration"),
            );
            assert!(error.is_err());
        }
    }

    #[test]
    fn failed_or_unconfirmed_moves_are_not_repeated() {
        let temp = tempfile::tempdir().unwrap();
        for response in [
            Err("refused".to_string()),
            Ok(json!({})),
            Ok(json!({"source":"codex", "hostId":"other", "threadId":THREAD, "projectId":PROJECT})),
        ] {
            let mut moves = 0;
            let result = assign_with_request(
                THREAD,
                temp.path(),
                Instant::now() + Duration::from_secs(5),
                |tool, _, _| match tool {
                    "list_projects" => Ok(json!({"projects":[project(temp.path())]})),
                    #[cfg(windows)]
                    "navigate_to_codex_page" if moves == 0 => Ok(json!({"navigated":true})),
                    #[cfg(windows)]
                    "read_thread" => Ok(
                        json!({"thread":{"id":THREAD, "kind":"codex", "hostId":"local",
                        "status":{"type":"active"}}}),
                    ),
                    "move_thread_to_project" => {
                        moves += 1;
                        response.clone()
                    }
                    _ => panic!("must stop after an unconfirmed placement"),
                },
                #[cfg(windows)]
                || panic!("existing project"),
            );
            assert!(result.is_err());
            assert_eq!(moves, 1);
        }
        assert!(assign_with_request(
            THREAD,
            temp.path(),
            Instant::now(),
            |_, _, _| { panic!("expired request must not run") },
            #[cfg(windows)]
            || panic!("expired request must not register")
        )
        .is_err());
    }

    #[cfg(windows)]
    #[test]
    fn waits_for_stored_history_without_waiting_for_an_idle_agent() {
        let temp = tempfile::tempdir().unwrap();
        let mut reads = 0;
        let mut moves = 0;
        assign_with_request(
            THREAD,
            temp.path(),
            Instant::now() + Duration::from_secs(5),
            |tool, args, _| match tool {
                "list_projects" => Ok(json!({"projects":[project(temp.path())]})),
                "navigate_to_codex_page" => Ok(json!({"navigated":true})),
                "read_thread" => {
                    reads += 1;
                    Ok(json!({"thread":{"id":THREAD, "kind":"codex", "hostId":"local",
                        "status":{"type":"notLoaded"}},
                        "turns":if reads == 1 { json!([]) } else { json!([{"id":"stored-turn"}]) }}))
                }
                "move_thread_to_project" => {
                    assert_eq!(reads, 2);
                    moves += 1;
                    Ok(args)
                }
                _ => panic!("unexpected native call"),
            },
            || panic!("existing project"),
        ).unwrap();
        assert_eq!(moves, 1);
    }

    #[cfg(windows)]
    #[test]
    fn unavailable_or_different_chats_are_never_moved() {
        let temp = tempfile::tempdir().unwrap();
        let ready = json!({"thread":{"id":THREAD, "kind":"codex", "hostId":"local",
            "status":{"type":"idle"}}});
        let mut responses = vec![
            Err("read failed".to_string()),
            Ok(json!({})),
            Ok(
                json!({"thread":{"id":THREAD, "kind":"codex", "hostId":"local",
                "status":{"type":"notLoaded"}}, "turns":[]}),
            ),
            Ok(
                json!({"thread":{"id":THREAD, "kind":"codex", "hostId":"local",
                "status":{"type":"notLoaded"}}}),
            ),
        ];
        for (key, value) in [
            ("id", "another-chat"),
            ("hostId", "remote"),
            ("kind", "chatgpt"),
        ] {
            let mut wrong = ready.clone();
            wrong["thread"][key] = json!(value);
            responses.push(Ok(wrong));
        }
        for status in ["systemError", "unknown"] {
            let mut unready = ready.clone();
            unready["thread"]["status"]["type"] = json!(status);
            responses.push(Ok(unready));
        }
        for response in responses {
            let result = assign_with_request(
                THREAD,
                temp.path(),
                Instant::now() + Duration::from_millis(20),
                |tool, _, _| match tool {
                    "list_projects" => Ok(json!({"projects":[project(temp.path())]})),
                    "navigate_to_codex_page" => Ok(json!({"navigated":true})),
                    "read_thread" => response.clone(),
                    _ => panic!(
                        "must not move a chat with an unconfirmed identity or unreadable status"
                    ),
                },
                || panic!("existing project"),
            );
            assert!(result.is_err());
        }
    }

    #[cfg(windows)]
    #[test]
    fn unconfirmed_navigation_never_reads_or_moves() {
        let temp = tempfile::tempdir().unwrap();
        for response in [
            Err("navigation failed".into()),
            Ok(json!({"navigated":false})),
            Ok(json!({})),
        ] {
            assert!(assign_with_request(
                THREAD,
                temp.path(),
                Instant::now() + Duration::from_secs(5),
                |tool, _, _| match tool {
                    "list_projects" => Ok(json!({"projects":[project(temp.path())]})),
                    "navigate_to_codex_page" => response.clone(),
                    _ => panic!("must stop after unconfirmed navigation"),
                },
                || panic!("existing project"),
            )
            .is_err());
        }
    }

    #[cfg(windows)]
    #[test]
    fn registers_a_missing_folder_once_and_waits_for_its_exact_native_project() {
        use std::cell::Cell;
        let temp = tempfile::tempdir().unwrap();
        let registrations = Cell::new(0);
        let mut lists = 0;
        let mut moves = 0;
        assign_with_request(
            THREAD,
            temp.path(),
            Instant::now() + Duration::from_secs(5),
            |tool, args, _| match tool {
                "list_projects" => {
                    lists += 1;
                    assert_eq!(registrations.get(), usize::from(lists > 1));
                    let mut projects = vec![project(&temp.path().join("child")),
                        json!({"projectId":"remote", "projectKind":"remote", "hostId":"remote", "path":temp.path()})];
                    if lists == 3 { projects.push(project(temp.path())); }
                    Ok(json!({"projects":projects}))
                }
                "navigate_to_codex_page" => Ok(json!({"navigated":true})),
                "read_thread" => Ok(json!({"thread":{"id":THREAD, "kind":"codex", "hostId":"local",
                    "status":{"type":"notLoaded"}}, "turns":[{"id":"stored-turn"}]})),
                "move_thread_to_project" => {
                    assert_eq!(lists, 3);
                    assert_eq!(args["projectId"], PROJECT);
                    moves += 1;
                    Ok(args)
                }
                _ => panic!("unexpected native call"),
            },
            || { registrations.set(registrations.get() + 1); Ok(()) },
        ).unwrap();
        assert_eq!(registrations.get(), 1);
        assert_eq!(moves, 1);
    }

    #[cfg(windows)]
    #[test]
    fn failed_or_unconfirmed_registration_never_moves_or_repeats_the_launch() {
        let temp = tempfile::tempdir().unwrap();
        for launch in [Err("launch failed".to_string()), Ok(())] {
            let mut registrations = 0;
            let error = assign_with_request(
                THREAD,
                temp.path(),
                Instant::now() + Duration::from_millis(20),
                |tool, _, _| {
                    assert_eq!(
                        tool, "list_projects",
                        "unregistered folders must not move or open a chat"
                    );
                    Ok(json!({"projects":[]}))
                },
                || {
                    registrations += 1;
                    launch.clone()
                },
            )
            .unwrap_err();
            assert_eq!(registrations, 1);
            assert!(error.contains(if launch.is_err() {
                "launch failed"
            } else {
                "in time"
            }));
        }
    }

    #[cfg(windows)]
    #[test]
    fn failed_or_invalid_project_queries_never_launch_registration() {
        let temp = tempfile::tempdir().unwrap();
        for response in [
            Err("profile changed".into()),
            Ok(json!({})),
            Ok(json!({"projects":vec![project(temp.path()); 10_001]})),
        ] {
            assert!(assign_with_request(
                THREAD,
                temp.path(),
                Instant::now() + Duration::from_secs(5),
                |tool, _, _| {
                    assert_eq!(tool, "list_projects");
                    response.clone()
                },
                || panic!("only a valid empty match can register a folder"),
            )
            .is_err());
        }
    }
}
