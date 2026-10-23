#[derive(Clone)]
pub struct Message {
    pub role: &'static str,
    pub content: String,
}

#[derive(Default)]
pub struct Chat {
    messages: Vec<Message>,
    response: String,
}

impl Chat {
    pub fn begin(&mut self, input: String) -> &[Message] {
        self.response.clear();
        self.messages.push(Message {
            role: "user",
            content: input,
        });
        &self.messages
    }

    pub fn append(&mut self, text: &str) {
        self.response.push_str(text);
    }

    pub fn finish(&mut self) {
        if self.response.is_empty() {
            // A failed or cancelled request with no answer should not leave an
            // unanswered turn in the context sent with the next request.
            self.messages.pop();
        } else {
            self.messages.push(Message {
                role: "assistant",
                content: std::mem::take(&mut self.response),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follow_up_contains_both_sides_of_previous_turn() {
        let mut chat = Chat::default();
        chat.begin("Remember blue".into());
        chat.append("I will ");
        chat.append("remember blue.");
        chat.finish();
        let messages = chat.begin("What color?".into());
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[1].role, "assistant");
        assert_eq!(messages[1].content, "I will remember blue.");
    }

    #[test]
    fn empty_cancelled_turn_is_removed_but_partial_reply_is_kept() {
        let mut chat = Chat::default();
        chat.begin("cancel".into());
        chat.finish();
        assert_eq!(chat.begin("retry".into()).len(), 1);
        chat.append("partial");
        chat.finish();
        assert_eq!(chat.begin("continue".into())[1].content, "partial");
    }
}
