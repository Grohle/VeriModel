//! A scripted backend for tests: returns canned answers in order.

use std::collections::VecDeque;

use super::{Backend, GenOptions};
use crate::error::{AssistantError, AssistantResult};

#[derive(Debug, Default)]
pub struct MockBackend {
    pub answers: VecDeque<String>,
    pub prompts: Vec<String>,
}

impl MockBackend {
    pub fn new(answers: impl IntoIterator<Item = impl Into<String>>) -> MockBackend {
        MockBackend {
            answers: answers.into_iter().map(Into::into).collect(),
            prompts: Vec::new(),
        }
    }
}

impl Backend for MockBackend {
    fn name(&self) -> &str {
        "mock"
    }
    fn generate(&mut self, prompt: &str, _opts: &GenOptions) -> AssistantResult<String> {
        self.prompts.push(prompt.to_string());
        self.answers
            .pop_front()
            .ok_or_else(|| AssistantError::Generation("mock backend has no more answers".into()))
    }
}
