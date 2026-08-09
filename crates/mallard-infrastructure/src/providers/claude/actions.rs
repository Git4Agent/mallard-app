//! Safe Claude session actions surfaced through the generic action contract.

use crate::providers::launch::{launch_terminal_command, shell_quote};
use crate::providers::traits::{
    SessionActionContext, SessionActionDescriptor, SessionActionKind, SessionActionOutcome,
    SessionActionRequest,
};

pub(super) const OPEN_IN_TERMINAL: &str = "claude.open-in-terminal";

fn validated_session_id(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err("Claude session id is not a safe resume argument".to_string());
    }
    Ok(())
}

pub(super) fn available_actions(
    context: &SessionActionContext<'_>,
) -> Vec<SessionActionDescriptor> {
    let resume_ready =
        context.is_local && validated_session_id(context.provider_session_id).is_ok();
    vec![SessionActionDescriptor {
        action_id: OPEN_IN_TERMINAL.to_string(),
        kind: SessionActionKind::OpenInTerminal,
        label: "Open in Terminal".to_string(),
        enabled: resume_ready,
        disabled_reason: (!resume_ready)
            .then(|| "This session is not materialized locally.".to_string()),
    }]
}

pub(super) fn execute_action(
    request: &mut SessionActionRequest<'_>,
) -> Result<SessionActionOutcome, String> {
    match request.action_id {
        OPEN_IN_TERMINAL => {
            validated_session_id(request.provider_session_id)?;
            if !request.project_root.is_absolute() || !request.provider_home.is_absolute() {
                return Err("project root and Claude home must be absolute".to_string());
            }
            let command = format!(
                "cd {} && CLAUDE_CONFIG_DIR={} claude --resume {}",
                shell_quote(&request.project_root.to_string_lossy()),
                shell_quote(&request.provider_home.to_string_lossy()),
                shell_quote(request.provider_session_id)
            );
            (request.before_launch)(&command);
            launch_terminal_command(&command)?;
            Ok(SessionActionOutcome::Launched)
        }
        other => Err(format!("unknown Claude session action '{}'", other)),
    }
}
