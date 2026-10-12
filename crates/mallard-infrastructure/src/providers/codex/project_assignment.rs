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
    register_project: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(30);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| "Could not start the Codex Desktop connection.".to_string())?;
    let mut desktop = runtime.block_on(windows::connect(executable, home, deadline))?;
    // A running Windows app can ignore a launch environment. Verify the
    // selected profile before sending the folder route that registers it.
    register_project()?;
    assign_with_request(thread_id, project_root, deadline, |tool, args, timeout| {
        runtime.block_on(desktop.call(thread_id, tool, args, timeout))
    })
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
    let project_id = loop {
        if let Some(project_id) = project_id_in_listing(
            request("list_projects", json!({}), remaining()?)?,
            project_root,
        )? {
            break project_id;
        }
        // The launcher sent the folder route once. Registration is asynchronous;
        // only repeat this read until the exact project appears.
        std::thread::sleep(Duration::from_millis(100).min(remaining()?));
    };

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
    fn moves_and_opens_the_same_old_chat_in_the_exact_local_project() {
        let temp = tempfile::tempdir().unwrap();
        let canonical = std::fs::canonicalize(temp.path()).unwrap();
        let ordinary = dunce::simplified(&canonical);
        let mut calls = Vec::new();
        assign_with_request(THREAD, &canonical, Instant::now() + Duration::from_secs(5),
            |tool, args, _| {
            calls.push(tool.to_string());
            match tool {
                "list_projects" => Ok(json!({"projects": [project(&ordinary.join("child")),
                    {"projectId":"remote", "projectKind":"remote", "hostId":"remote", "path":ordinary},
                    project(ordinary)]})),
                "move_thread_to_project" => {
                    assert_eq!(args, json!({"source":"codex", "hostId":"local", "threadId":THREAD, "projectId":PROJECT}));
                    Ok(args)
                }
                "navigate_to_codex_page" => {
                    assert_eq!(args, json!({"threadId":THREAD}));
                    Ok(json!({"navigated":true}))
                }
                _ => panic!("unexpected native call"),
            }
        }).unwrap();
        assert_eq!(
            calls,
            [
                "list_projects",
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
                    _ => panic!("unexpected native call"),
                }
            },
        )
        .unwrap();
        assert_eq!(
            calls,
            [
                "list_projects",
                "list_projects",
                "list_projects",
                "list_projects",
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
        )
        .unwrap_err();
        assert!(error.contains("in time"), "{error}");
    }

    #[test]
    fn ambiguous_and_invalid_projects_never_move_a_chat() {
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
            );
            assert!(error.is_err());
        }
    }

    #[test]
    fn failed_or_unconfirmed_moves_are_not_repeated_or_opened() {
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
                    "move_thread_to_project" => {
                        moves += 1;
                        response.clone()
                    }
                    _ => panic!("must not open an unconfirmed placement"),
                },
            );
            assert!(result.is_err());
            assert_eq!(moves, 1);
        }
        assert!(
            assign_with_request(THREAD, temp.path(), Instant::now(), |_, _, _| {
                panic!("expired request must not run")
            })
            .is_err()
        );
    }
}
