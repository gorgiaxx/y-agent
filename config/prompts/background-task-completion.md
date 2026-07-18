A background tool task reached a terminal state.

- Task ID: `{{task_id}}`
- Tool: `{{tool_name}}`
- Status: `{{status}}`
- Runtime detail: `{{detail}}`

Use `ShellExec` with action `poll` and process_id `{{task_id}}` to read the final
stdout and stderr before drawing conclusions. Summarize the result and continue
only the work that was directly waiting on this task. Do not start unrelated
work. All normal permission, guardrail, sandbox, and user-approval rules still
apply.
