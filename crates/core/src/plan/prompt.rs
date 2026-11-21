pub const EXPLAIN_ANALYSIS_PROMPT: &str = include_str!("../../prompts/ai_explain_analysis_ru.md");

pub fn explain_analysis_prompt() -> &'static str {
    EXPLAIN_ANALYSIS_PROMPT
}
