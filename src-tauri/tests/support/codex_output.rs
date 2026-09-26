use serde_json::Value;

// Codex 0.156.0 emits config and missing-model-metadata startup warnings as
// completed error items. Accept only these warnings before the model turn;
// keep runtime errors fatal.
pub fn is_startup_warning(event: &Value, turn_started: bool, model: &str) -> bool {
    if turn_started || event["type"] != "item.completed" || event["item"]["type"] != "error" {
        return false;
    }
    let Some(message) = event["item"]["message"].as_str() else {
        return false;
    };
    if message == format!(
        "Model metadata for `{model}` not found. Defaulting to fallback metadata; this can degrade performance and cause issues."
    ) {
        return true;
    }
    let Some(rest) = message.strip_prefix("Codex is ignoring ") else {
        return false;
    };
    let Some((count, rest)) = rest.split_once(" unrecognized configuration setting") else {
        return false;
    };
    let Ok(count) = count.parse::<usize>() else {
        return false;
    };
    let suffix = if count == 1 { "" } else { "s" };
    count > 0
        && rest.starts_with(&format!(
            "{suffix}. Check for typos or deprecated settings.\n"
        ))
}

#[test]
fn config_warning_does_not_allow_runtime_errors_or_tools() {
    let mut event = serde_json::json!({
        "type": "item.completed",
        "item": {
            "type": "error",
            "message": "Codex is ignoring 1 unrecognized configuration setting. Check for typos or deprecated settings.\n`unknown_setting` is ignored."
        }
    });
    assert!(is_startup_warning(&event, false, "gpt-6-luna"));
    assert!(!is_startup_warning(&event, true, "gpt-6-luna"));
    event["item"]["type"] = "command_execution".into();
    assert!(!is_startup_warning(&event, false, "gpt-6-luna"));
    event["item"]["type"] = "error".into();
    for message in [
        "request failed",
        "rate limit exceeded",
        "Codex is ignoring an error",
    ] {
        event["item"]["message"] = message.into();
        assert!(!is_startup_warning(&event, false, "gpt-6-luna"));
    }
}

#[test]
fn model_metadata_warning_requires_exact_model_message_and_startup_phase() {
    let message = "Model metadata for `gpt-6-luna` not found. Defaulting to fallback metadata; this can degrade performance and cause issues.";
    let mut event = serde_json::json!({
        "type": "item.completed",
        "item": { "type": "error", "message": message }
    });
    assert!(is_startup_warning(&event, false, "gpt-6-luna"));
    assert!(!is_startup_warning(&event, true, "gpt-6-luna"));
    assert!(!is_startup_warning(&event, false, "gpt-5.6-luna"));
    event["item"]["message"] = format!("{message} Request failed.").into();
    assert!(!is_startup_warning(&event, false, "gpt-6-luna"));
    event["item"]["message"] = message.into();
    event["item"]["type"] = "command_execution".into();
    assert!(!is_startup_warning(&event, false, "gpt-6-luna"));
    event["item"]["type"] = "error".into();
    event["type"] = "error".into();
    assert!(!is_startup_warning(&event, false, "gpt-6-luna"));
}
