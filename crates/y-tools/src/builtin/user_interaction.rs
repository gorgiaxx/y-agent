//! `AskUser` built-in tool: ask the user multiple-choice questions.
//!
//! Inspired by Cursor's `AskUserQuestion` tool. This tool enables the LLM to:
//! 1. Gather user preferences or requirements
//! 2. Clarify ambiguous instructions
//! 3. Get decisions on implementation choices during execution
//! 4. Offer choices about what direction to take
//!
//! The `execute()` method validates input and returns a pending descriptor
//! containing the structured questions. Actual user interaction is performed
//! by the `UserInteractionOrchestrator` in `y-service`, which intercepts
//! `AskUser` calls and routes them to the appropriate UI layer (CLI/TUI/GUI).
//!
//! Users always have the implicit ability to select "Other" and provide
//! free-text input -- this is enforced by the UI layer, not by this tool.

use async_trait::async_trait;

use y_core::runtime::RuntimeCapability;
use y_core::tool::{
    Tool, ToolCategory, ToolDefinition, ToolError, ToolInput, ToolOutput, ToolType,
};
use y_core::types::ToolName;

/// Maximum number of questions per invocation.
const MAX_QUESTIONS: usize = 4;

/// Maximum number of options per question.
const MAX_OPTIONS: usize = 4;

/// Minimum number of options per question.
const MIN_OPTIONS: usize = 2;

/// Built-in tool for asking the user multiple-choice questions.
///
/// When invoked by the LLM, this tool validates the question structure and
/// returns a pending descriptor. The orchestrator layer intercepts the call,
/// presents the questions to the user via the active UI, collects answers,
/// and returns them as the tool result to the LLM.
pub struct AskUserTool {
    def: ToolDefinition,
}

impl AskUserTool {
    /// Create a new `AskUser` tool instance.
    pub fn new() -> Self {
        Self {
            def: Self::tool_definition(),
        }
    }

    /// The tool definition for `AskUser`.
    pub fn tool_definition() -> ToolDefinition {
        ToolDefinition {
            name: ToolName::from_string("AskUser"),
            description: "Ask the user multiple-choice questions to gather information, \
                clarify ambiguity, understand preferences, make decisions, \
                or offer choices."
                .into(),
            help: Some(
                "Ask the user 1-4 multiple-choice questions during execution.\n\
                 \n\
                 Usage notes:\n\
                 - Users can always select \"Other\" to provide custom text input\n\
                 - Use multi_select: true to allow multiple answers per question\n\
                 - If you recommend a specific option, make it the first option \
                   and add \"(Recommended)\" at the end\n\
                 \n\
                 Example:\n\
                 AskUser({\n\
                   \"questions\": [{\n\
                     \"question\": \"Which library should we use for date formatting?\",\n\
                     \"options\": [\"chrono (Recommended)\", \"time\"]\n\
                   }]\n\
                 })"
                .into(),
            ),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "questions": {
                        "type": "array",
                        "description": "Questions to ask the user (1-4 questions)",
                        "minItems": 1,
                        "maxItems": MAX_QUESTIONS,
                        "items": {
                            "type": "object",
                            "properties": {
                                "question": {
                                    "type": "string",
                                    "description": "The complete question text. Should be clear, \
                                        specific, and end with a question mark."
                                },
                                "options": {
                                    "type": "array",
                                    "description": "Available choices (2-4 short labels). There \
                                        should be no 'Other' option; it is provided automatically.",
                                    "minItems": MIN_OPTIONS,
                                    "maxItems": MAX_OPTIONS,
                                    "items": {
                                        "type": "string",
                                        "description": "Display text (1-5 words) the \
                                            user will see and select."
                                    }
                                },
                                "multi_select": {
                                    "type": "boolean",
                                    "description": "When true, the user can select multiple \
                                        options. Default: false.",
                                    "default": false
                                }
                            },
                            "required": ["question", "options"]
                        }
                    }
                },
                "required": ["questions"]
            }),
            result_schema: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "questions": {
                        "type": "array",
                        "description": "The questions that were asked"
                    },
                    "answers": {
                        "type": "object",
                        "description": "User answers keyed by question text. \
                            Multi-select answers are comma-separated.",
                        "additionalProperties": { "type": "string" }
                    }
                },
                "required": ["questions", "answers"]
            })),
            category: ToolCategory::Interaction,
            tool_type: ToolType::BuiltIn,
            capabilities: RuntimeCapability::default(),
            is_dangerous: false,
        }
    }

    /// Validate the question structure and return detailed errors.
    ///
    /// Public accessor for use by the `UserInteractionOrchestrator` in
    /// `y-service`, which intercepts `AskUser` calls before they reach
    /// the tool's `execute()` method.
    pub fn validate_questions_public(questions: &serde_json::Value) -> Result<(), ToolError> {
        Self::validate_questions(questions)
    }

    /// Validate the question structure and return detailed errors.
    fn validate_questions(questions: &serde_json::Value) -> Result<(), ToolError> {
        let arr = questions
            .as_array()
            .ok_or_else(|| ToolError::ValidationError {
                message: "'questions' must be an array".into(),
            })?;

        if arr.is_empty() || arr.len() > MAX_QUESTIONS {
            return Err(ToolError::ValidationError {
                message: format!(
                    "'questions' must contain 1-{MAX_QUESTIONS} items, got {}",
                    arr.len()
                ),
            });
        }

        // Check question text uniqueness.
        let mut seen_questions = std::collections::HashSet::new();
        for (i, q) in arr.iter().enumerate() {
            let question_text = q.get("question").and_then(|v| v.as_str()).ok_or_else(|| {
                ToolError::ValidationError {
                    message: format!("questions[{i}]: missing 'question' text"),
                }
            })?;

            if !seen_questions.insert(question_text) {
                return Err(ToolError::ValidationError {
                    message: format!("duplicate question text: \"{question_text}\""),
                });
            }

            let options = q.get("options").and_then(|v| v.as_array()).ok_or_else(|| {
                ToolError::ValidationError {
                    message: format!("questions[{i}]: missing or invalid 'options' array"),
                }
            })?;

            if options.len() < MIN_OPTIONS || options.len() > MAX_OPTIONS {
                return Err(ToolError::ValidationError {
                    message: format!(
                        "questions[{i}]: 'options' must have {MIN_OPTIONS}-{MAX_OPTIONS} items, \
                         got {}",
                        options.len()
                    ),
                });
            }

            // Check option label uniqueness within each question.
            let mut seen_labels = std::collections::HashSet::new();
            for (j, opt) in options.iter().enumerate() {
                let label = opt.as_str().ok_or_else(|| ToolError::ValidationError {
                    message: format!("questions[{i}].options[{j}]: must be a string"),
                })?;

                if !seen_labels.insert(label) {
                    return Err(ToolError::ValidationError {
                        message: format!("questions[{i}]: duplicate option \"{label}\""),
                    });
                }
            }
        }

        Ok(())
    }

    /// Format collected answers into a human-readable string for the LLM.
    ///
    /// This is used by the orchestrator to convert user answers into a text
    /// representation that gets returned as the tool result to the model.
    pub fn format_answers_for_llm(answers: &serde_json::Map<String, serde_json::Value>) -> String {
        let formatted: Vec<String> = answers
            .iter()
            .map(|(question, answer)| {
                let answer_str = answer.as_str().unwrap_or("");
                format!("\"{question}\"=\"{answer_str}\"")
            })
            .collect();

        format!(
            "User has answered your questions: {}. \
             You can now continue with the user's answers in mind.",
            formatted.join(", ")
        )
    }
}

impl Default for AskUserTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for AskUserTool {
    async fn execute(&self, input: ToolInput) -> Result<ToolOutput, ToolError> {
        let questions =
            input
                .arguments
                .get("questions")
                .ok_or_else(|| ToolError::ValidationError {
                    message: "'questions' parameter is required".into(),
                })?;

        // Deep validation of question structure.
        Self::validate_questions(questions)?;

        // Extract optional pre-filled answers (injected by the orchestrator
        // after collecting user responses).
        let answers = input
            .arguments
            .get("answers")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));

        let annotations = input.arguments.get("annotations").cloned();

        // Return a descriptor for the orchestrator to intercept.
        // When answers are empty, the orchestrator presents the questions to
        // the user. When answers are populated, this is the post-interaction
        // pass-through.
        let mut content = serde_json::json!({
            "action": "AskUser",
            "questions": questions,
            "answers": answers,
            "status": if answers.as_object().is_some_and(|m| !m.is_empty()) {
                "answered"
            } else {
                "pending"
            }
        });

        if let Some(ann) = annotations {
            content["annotations"] = ann;
        }

        Ok(ToolOutput {
            success: true,
            content,
            warnings: vec![],
            metadata: serde_json::json!({}),
        })
    }

    fn definition(&self) -> &ToolDefinition {
        &self.def
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use y_core::types::SessionId;

    fn make_input(args: serde_json::Value) -> ToolInput {
        ToolInput {
            call_id: "call_001".into(),
            name: ToolName::from_string("AskUser"),
            arguments: args,
            session_id: SessionId::new(),
            working_dir: None,
            command_runner: None,
        }
    }

    fn valid_questions() -> serde_json::Value {
        serde_json::json!({
            "questions": [{
                "question": "Which library should we use for date formatting?",
                "options": ["chrono (Recommended)", "time"]
            }]
        })
    }

    #[tokio::test]
    async fn test_ask_user_single_question_pending() {
        let tool = AskUserTool::new();
        let input = make_input(valid_questions());
        let output = tool.execute(input).await.unwrap();

        assert!(output.success);
        assert_eq!(output.content["action"], "AskUser");
        assert_eq!(output.content["status"], "pending");

        let questions = output.content["questions"].as_array().unwrap();
        assert_eq!(questions.len(), 1);
        assert_eq!(
            questions[0]["question"],
            "Which library should we use for date formatting?"
        );
    }

    #[tokio::test]
    async fn test_ask_user_with_answers() {
        let tool = AskUserTool::new();
        let mut args = valid_questions();
        args["answers"] = serde_json::json!({
            "Which library should we use for date formatting?": "chrono (Recommended)"
        });
        let input = make_input(args);
        let output = tool.execute(input).await.unwrap();

        assert!(output.success);
        assert_eq!(output.content["status"], "answered");
        assert_eq!(
            output.content["answers"]["Which library should we use for date formatting?"],
            "chrono (Recommended)"
        );
    }

    #[tokio::test]
    async fn test_ask_user_multiple_questions() {
        let tool = AskUserTool::new();
        let input = make_input(serde_json::json!({
            "questions": [
                {
                    "question": "Which framework?",
                    "options": ["Axum", "Actix-web"]
                },
                {
                    "question": "Which database?",
                    "options": ["SQLite", "PostgreSQL"]
                }
            ]
        }));
        let output = tool.execute(input).await.unwrap();

        assert!(output.success);
        let questions = output.content["questions"].as_array().unwrap();
        assert_eq!(questions.len(), 2);
    }

    #[tokio::test]
    async fn test_ask_user_with_multi_select() {
        let tool = AskUserTool::new();
        let input = make_input(serde_json::json!({
            "questions": [{
                "question": "Which features to enable?",
                "multi_select": true,
                "options": ["Logging", "Metrics", "Tracing"]
            }]
        }));
        let output = tool.execute(input).await.unwrap();
        assert!(output.success);
    }

    #[tokio::test]
    async fn test_ask_user_with_string_options() {
        let tool = AskUserTool::new();
        let input = make_input(serde_json::json!({
            "questions": [{
                "question": "Which layout do you prefer?",
                "options": ["Sidebar", "Top bar"]
            }]
        }));
        let output = tool.execute(input).await.unwrap();
        assert!(output.success);

        let options = output.content["questions"][0]["options"]
            .as_array()
            .unwrap();
        assert_eq!(options[0].as_str().unwrap(), "Sidebar");
    }

    #[tokio::test]
    async fn test_ask_user_missing_questions() {
        let tool = AskUserTool::new();
        let input = make_input(serde_json::json!({}));
        let result = tool.execute(input).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ToolError::ValidationError { .. }
        ));
    }

    #[tokio::test]
    async fn test_ask_user_empty_questions() {
        let tool = AskUserTool::new();
        let input = make_input(serde_json::json!({"questions": []}));
        let result = tool.execute(input).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_ask_user_too_many_questions() {
        let tool = AskUserTool::new();
        let questions: Vec<serde_json::Value> = (0..5)
            .map(|i| {
                serde_json::json!({
                    "question": format!("Question {i}?"),
                    "options": ["Yes", "No"]
                })
            })
            .collect();
        let input = make_input(serde_json::json!({"questions": questions}));
        let result = tool.execute(input).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_ask_user_too_few_options() {
        let tool = AskUserTool::new();
        let input = make_input(serde_json::json!({
            "questions": [{
                "question": "Pick one?",
                "options": ["Only"]
            }]
        }));
        let result = tool.execute(input).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_ask_user_too_many_options() {
        let tool = AskUserTool::new();
        let options: Vec<serde_json::Value> = (0..5)
            .map(|i| serde_json::json!(format!("Option {i}")))
            .collect();
        let input = make_input(serde_json::json!({
            "questions": [{
                "question": "Pick?",
                "options": options
            }]
        }));
        let result = tool.execute(input).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_ask_user_duplicate_question_text() {
        let tool = AskUserTool::new();
        let input = make_input(serde_json::json!({
            "questions": [
                {
                    "question": "Same question?",
                    "options": ["A", "B"]
                },
                {
                    "question": "Same question?",
                    "options": ["C", "D"]
                }
            ]
        }));
        let result = tool.execute(input).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_ask_user_duplicate_option_labels() {
        let tool = AskUserTool::new();
        let input = make_input(serde_json::json!({
            "questions": [{
                "question": "Pick?",
                "options": ["Same", "Same"]
            }]
        }));
        let result = tool.execute(input).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_ask_user_non_string_option() {
        let tool = AskUserTool::new();
        let input = make_input(serde_json::json!({
            "questions": [{
                "question": "Pick?",
                "options": [123, "B"]
            }]
        }));
        let result = tool.execute(input).await;
        assert!(result.is_err());
    }

    #[test]
    fn test_ask_user_definition() {
        let def = AskUserTool::tool_definition();
        assert_eq!(def.name.as_str(), "AskUser");
        assert_eq!(def.category, ToolCategory::Interaction);
        assert_eq!(def.tool_type, ToolType::BuiltIn);
        assert!(!def.is_dangerous);
        assert!(def.result_schema.is_some());

        let props = def.parameters["properties"].as_object().unwrap();
        assert!(props.contains_key("questions"));
        // header should no longer be in the schema.
        let q_props = def.parameters["properties"]["questions"]["items"]["properties"]
            .as_object()
            .unwrap();
        assert!(!q_props.contains_key("header"));

        let required = def.parameters["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v == "questions"));
    }

    #[test]
    fn test_format_answers_simple() {
        let mut answers = serde_json::Map::new();
        answers.insert(
            "Which library?".into(),
            serde_json::Value::String("chrono".into()),
        );

        let result = AskUserTool::format_answers_for_llm(&answers);
        assert!(result.contains("\"Which library?\"=\"chrono\""));
        assert!(result.contains("You can now continue"));
    }
}
