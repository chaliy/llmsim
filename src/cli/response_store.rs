// Completed HTTP responses keep a bounded snapshot of the effective history.
// A continuation can fork from any retained response, even after its ancestors
// have been evicted. The byte budget includes both the body and its history.

use crate::openai::{
    ContentPart, InputItem, InputRole, MessageContent, OutputContentPart, OutputItem,
    ResponsesInput, ResponsesRequest, ResponsesResponse,
};
use crate::responses_stream::ResponseCompleted;
use axum::body::Bytes;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

const MAX_RESPONSES: usize = 1_000;
const MAX_BYTES: usize = 32 * 1024 * 1024;

#[derive(Default)]
pub(super) struct ResponseStore {
    cache: Mutex<Cache>,
}

#[derive(Default)]
struct Cache {
    entries: VecDeque<Entry>,
    bytes: usize,
}

struct Entry {
    id: String,
    input: Bytes,
    response: Bytes,
}

impl Entry {
    fn size(&self) -> usize {
        self.input.len() + self.response.len()
    }
}

impl ResponseStore {
    pub(super) fn save(&self, request: &ResponsesRequest, response: &ResponsesResponse) {
        if request.store != Some(false) {
            self.insert(&request.input, response);
        }
    }

    pub(super) fn on_completed(
        self: &Arc<Self>,
        request: &ResponsesRequest,
    ) -> Option<ResponseCompleted> {
        if !request.stream || request.store == Some(false) {
            return None;
        }
        let store = self.clone();
        let input = request.input.clone();
        Some(Box::new(move |response| store.insert(&input, response)))
    }

    fn insert(&self, input: &ResponsesInput, response: &ResponsesResponse) {
        let (Ok(input), Ok(body)) = (serde_json::to_vec(input), serde_json::to_vec(response))
        else {
            return;
        };
        let entry = Entry {
            id: response.id.clone(),
            input: input.into(),
            response: body.into(),
        };
        let size = entry.size();
        if size > MAX_BYTES {
            return;
        }
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        while cache.entries.len() >= MAX_RESPONSES || cache.bytes + size > MAX_BYTES {
            let oldest = cache
                .entries
                .pop_front()
                .expect("capacity requires an entry");
            cache.bytes -= oldest.size();
        }
        cache.bytes += size;
        cache.entries.push_back(entry);
    }

    pub(super) fn get(&self, id: &str) -> Option<Bytes> {
        let cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        Some(
            cache
                .entries
                .iter()
                .find(|entry| entry.id == id)?
                .response
                .clone(),
        )
    }

    pub(super) fn delete(&self, id: &str) -> bool {
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        let Some(index) = cache.entries.iter().position(|entry| entry.id == id) else {
            return false;
        };
        let entry = cache.entries.remove(index).expect("entry index is valid");
        cache.bytes -= entry.size();
        true
    }

    pub(super) fn continue_input(
        &self,
        id: &str,
        input: &ResponsesInput,
    ) -> Option<ResponsesInput> {
        let (prior, response) = {
            let cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
            let entry = cache.entries.iter().find(|entry| entry.id == id)?;
            (entry.input.clone(), entry.response.clone())
        };
        let mut history = into_items(serde_json::from_slice(&prior).ok()?);
        let response: ResponsesResponse = serde_json::from_slice(&response).ok()?;
        history.extend(response.output.into_iter().map(output_as_input));
        history.extend(into_items(input.clone()));
        Some(ResponsesInput::Items(history))
    }
}

fn into_items(input: ResponsesInput) -> Vec<InputItem> {
    match input {
        ResponsesInput::Items(items) => items,
        ResponsesInput::Text(text) => vec![InputItem::Message {
            role: InputRole::User,
            content: MessageContent::Text(text),
        }],
    }
}

fn output_as_input(output: OutputItem) -> InputItem {
    match output {
        OutputItem::Message { content, .. } => InputItem::Message {
            role: InputRole::Assistant,
            content: MessageContent::Parts(
                content
                    .into_iter()
                    .map(|part| {
                        let text = match part {
                            OutputContentPart::OutputText { text, .. } => text,
                            OutputContentPart::Refusal { refusal } => refusal,
                        };
                        ContentPart::InputText { text }
                    })
                    .collect(),
            ),
        },
        OutputItem::FunctionCall {
            call_id,
            name,
            arguments,
            ..
        } => InputItem::FunctionCall {
            call_id,
            name,
            arguments,
        },
        OutputItem::Reasoning { id, .. } => InputItem::Reasoning { id: Some(id) },
    }
}
