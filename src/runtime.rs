use anyhow::Result;

use crate::conversation::{Conversation, Message};
use crate::input::TerminalInput;
use crate::llm::BifrostClient;
use crate::output::TerminalOutput;
use crate::store::Store;

pub struct ChatRuntime {
    input: TerminalInput,
    output: TerminalOutput,
    llm: BifrostClient,
    store: Store,
    conversation_id: String,
    conversation: Conversation,
}

impl ChatRuntime {
    pub fn new(
        input: TerminalInput,
        output: TerminalOutput,
        llm: BifrostClient,
        store: Store,
        conversation_id: String,
        messages: Vec<Message>,
    ) -> Self {
        Self {
            input,
            output,
            llm,
            store,
            conversation_id,
            conversation: Conversation::from_messages(messages),
        }
    }

    pub async fn run(&mut self) -> Result<()> {
        loop {
            let Some(input) = self.input.read("> ")? else {
                self.output.newline();
                return Ok(());
            };

            let input = input.trim();
            if input.is_empty() {
                continue;
            }

            let parent_message_id = self.conversation.last_message_id().map(str::to_string);
            let user_message_id = self.store.save_message(
                &self.conversation_id,
                parent_message_id.as_deref(),
                "user",
                input,
                None,
            )?;
            self.conversation.add_message(Message {
                id: Some(user_message_id.clone()),
                parent_message_id,
                role: "user".to_string(),
                content: input.to_string(),
                metadata: None,
            });

            self.output.start_assistant_message();
            let reply = self
                .llm
                .stream(self.conversation.messages(), |text| {
                    self.output.assistant_delta(text)
                })
                .await?;
            self.output.end_assistant_message();
            let assistant_message_id = self.store.save_message(
                &self.conversation_id,
                Some(&user_message_id),
                "assistant",
                &reply,
                None,
            )?;
            self.conversation.add_message(Message {
                id: Some(assistant_message_id),
                parent_message_id: Some(user_message_id),
                role: "assistant".to_string(),
                content: reply,
                metadata: None,
            });
        }
    }
}
