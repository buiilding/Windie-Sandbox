use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    #[serde(skip_serializing)]
    pub id: Option<String>,
    #[serde(skip_serializing)]
    #[allow(dead_code)]
    pub parent_message_id: Option<String>,
    pub role: String,
    pub content: String,
    #[serde(skip_serializing)]
    #[allow(dead_code)]
    pub metadata: Option<String>,
}

#[derive(Debug, Default)]
pub struct Conversation {
    messages: Vec<Message>,
}

impl Conversation {
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_messages(messages: Vec<Message>) -> Self {
        Self { messages }
    }

    pub fn messages(&self) -> &[Message] {
        &self.messages
    }

    pub fn last_message_id(&self) -> Option<&str> {
        self.messages.last()?.id.as_deref()
    }

    pub fn add_message(&mut self, message: Message) {
        self.messages.push(message);
    }

    #[allow(dead_code)]
    pub fn add_user_message(&mut self, content: impl Into<String>) {
        self.messages.push(Message {
            id: None,
            parent_message_id: None,
            role: "user".to_string(),
            content: content.into(),
            metadata: None,
        });
    }

    #[allow(dead_code)]
    pub fn add_assistant_message(&mut self, content: impl Into<String>) {
        self.messages.push(Message {
            id: None,
            parent_message_id: None,
            role: "assistant".to_string(),
            content: content.into(),
            metadata: None,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_conversation_starts_empty() {
        let conversation = Conversation::new();

        assert!(conversation.messages().is_empty());
    }

    #[test]
    fn adds_user_message() {
        let mut conversation = Conversation::new();

        conversation.add_user_message("hello");

        assert_eq!(conversation.messages().len(), 1);
        assert_eq!(conversation.messages()[0].role, "user");
        assert_eq!(conversation.messages()[0].content, "hello");
    }

    #[test]
    fn adds_assistant_message() {
        let mut conversation = Conversation::new();

        conversation.add_assistant_message("hello back");

        assert_eq!(conversation.messages().len(), 1);
        assert_eq!(conversation.messages()[0].role, "assistant");
        assert_eq!(conversation.messages()[0].content, "hello back");
    }

    #[test]
    fn preserves_message_order() {
        let mut conversation = Conversation::new();

        conversation.add_user_message("one");
        conversation.add_assistant_message("two");
        conversation.add_user_message("three");

        assert_eq!(conversation.messages()[0].content, "one");
        assert_eq!(conversation.messages()[1].content, "two");
        assert_eq!(conversation.messages()[2].content, "three");
    }

    #[test]
    fn serializes_only_model_fields() {
        let message = Message {
            id: Some("message-id".to_string()),
            parent_message_id: Some("parent-id".to_string()),
            role: "user".to_string(),
            content: "hello".to_string(),
            metadata: Some(r#"{"tool_calls":[]}"#.to_string()),
        };

        let value = serde_json::to_value(message).unwrap();

        assert_eq!(
            value,
            serde_json::json!({"role": "user", "content": "hello"})
        );
    }
}
