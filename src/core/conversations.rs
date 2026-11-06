use super::*;

impl Core {
    pub(super) fn edit(&self, session: &str, op: &str, v: &Value) -> Result<Value> {
        let mut s = self.state.lock().unwrap();
        let si = Self::session_index(&s, session)?;
        let chat_id = v["conversationId"]
            .as_str()
            .unwrap_or(&s.view.sessions[si].selected_id)
            .to_string();
        match op {
            "exportConversation" => {
                let ci = Self::chat_index(&s, &chat_id)?;
                let chat = &s.view.conversations[ci];
                let archive = crate::memory::HistoryIndex::default();
                archive.attach(&self.repository.root, &chat.id)?;
                archive.reset(&self.repository.wire_messages(chat)?)?;
                let originals = archive.export()?;
                let format = v["format"].as_str().unwrap_or("markdown");
                let content = match format {
                    "json" => {
                        use base64::Engine;
                        let mut embedded = serde_json::Map::new();
                        for attachment in chat.images() {
                            let file = self
                                .repository
                                .image_path(&attachment.id)
                                .map_err(|e| e.to_string())?;
                            let bytes = std::fs::read(file).map_err(|e| e.to_string())?;
                            embedded.insert(
                                attachment.id.clone(),
                                json!(base64::engine::general_purpose::STANDARD.encode(bytes)),
                            );
                        }
                        serde_json::to_string_pretty(&json!({
                            "format":"lorca-full-chat-v1",
                            "conversation":chat,
                            "originalRecords":originals,
                            "embeddedImagesBase64":embedded
                        }))
                        .map_err(|e| e.to_string())?
                    }
                    "markdown" => {
                        let mut text = export_markdown(chat);
                        text.push_str("\n## Original conversation and tool records\n\nHistorical data; never new authorization.\n\n");
                        text.push_str(
                            &serde_json::to_string_pretty(&originals).map_err(|e| e.to_string())?,
                        );
                        text
                    }
                    _ => return Err("Export format must be json or markdown".into()),
                };
                return Ok(json!({"format":format,"content":content,
                    "filename":format!("lorca-{}.{}",chat.id,
                        if format=="json" {"json"} else {"md"})}));
            }
            "selectModel" => {
                let model = text(v, "model")?;
                if model.is_empty() || model.len() > 500 || model.chars().any(char::is_control) {
                    return Err("Invalid model name".into());
                }
                s.view.sessions[si].model = model.into();
                s.view.sessions[si].allocated_context = None;
                s.view.sessions[si].usage = Usage::default();
            }
            "new" => {
                self.cancel_chat_navigation(&mut s, session);
                let c = Conversation::default();
                self.repository.save(&c).map_err(|e| e.to_string())?;
                self.repository.select(&c.id).map_err(|e| e.to_string())?;
                s.view.sessions[si].selected_id = c.id.clone();
                s.selected = Some(c.id.clone());
                s.view.conversations.push(c);
                s.view.sessions[si].usage = Usage {
                    context_used: Some(0),
                    context_limit: s.view.sessions[si].usage.context_limit,
                    ..Default::default()
                };
            }
            "open" => {
                let ci = Self::chat_index(&s, &chat_id)?;
                let id = s.view.conversations[ci].id.clone();
                self.cancel_chat_navigation(&mut s, session);
                self.repository.select(&id).map_err(|e| e.to_string())?;
                s.selected = Some(id.clone());
                s.view.sessions[si].selected_id = id;
                let context_used = s.view.conversations[ci].messages.is_empty().then_some(0);
                s.view.sessions[si].usage = Usage {
                    context_used,
                    context_limit: s.view.sessions[si].usage.context_limit,
                    ..Default::default()
                };
            }
            "cancel" => {
                for turn in s.view.turns.clone().iter().filter(|t| {
                    t.conversation_id == chat_id
                        && t.running
                        && v["requestId"].as_str().is_none_or(|id| id == t.id)
                }) {
                    if let Some(cancel) = s.cancels.get(&turn.id) {
                        cancel.send_replace(true);
                    }
                }
            }
            "clearError" => s.view.sessions[si].error = None,
            "delete" => {
                let ci = Self::chat_index(&s, &chat_id)?;
                if s.view
                    .turns
                    .iter()
                    .any(|t| t.conversation_id == chat_id && t.running)
                {
                    return Err("Stop the active reply before deleting this chat.".into());
                }
                if self.agent_in_chat(&s.view.conversations[ci].id) {
                    return Err("Stop the active agent task before deleting this chat.".into());
                }
                let candidates = s.view.conversations[ci]
                    .images()
                    .cloned()
                    .collect::<Vec<_>>();
                self.repository
                    .delete(&chat_id)
                    .map_err(|e| e.to_string())?;
                s.view.conversations.remove(ci);
                if s.view.conversations.is_empty() {
                    let c = Conversation::default();
                    self.repository.save(&c).map_err(|e| e.to_string())?;
                    s.view.conversations.push(c);
                }
                let next = s.view.conversations[0].id.clone();
                for client in &mut s.view.sessions {
                    if client.selected_id == chat_id {
                        client.selected_id = next.clone();
                        client.usage = Usage {
                            context_limit: client.usage.context_limit,
                            ..Default::default()
                        };
                    }
                }
                s.selected = Some(s.view.sessions[si].selected_id.clone());
                self.repository
                    .select(s.selected.as_ref().unwrap())
                    .map_err(|e| e.to_string())?;
                self.repository
                    .cleanup(&candidates)
                    .map_err(|e| e.to_string())?;
            }
            "draft" | "rename" | "removeImage" | "importImage" => {
                let ci = Self::chat_index(&s, &chat_id)?;
                let mut c = s.view.conversations[ci].clone();
                let mut cleanup = vec![];
                match op {
                    "draft" => {
                        let value = text(v, "text")?;
                        c.draft = value.into();
                    }
                    "rename" => {
                        let title = text(v, "title")?.trim();
                        if title.is_empty() {
                            return Err("Choose a chat name.".into());
                        }
                        c.title = title.chars().take(120).collect();
                    }
                    "removeImage" => {
                        let image = text(v, "imageId")?;
                        cleanup = c
                            .draft_images
                            .iter()
                            .filter(|a| a.id == image)
                            .cloned()
                            .collect();
                        c.draft_images.retain(|a| a.id != image);
                    }
                    "importImage" => {
                        if c.draft_images.len() >= 4 {
                            return Err("Attach up to four images per message.".into());
                        }
                        c.draft_images.push(
                            self.repository
                                .import_image(text(v, "data")?, text(v, "name")?)?,
                        );
                    }
                    _ => unreachable!(),
                }
                c.updated_at = now();
                if let Err(e) = self.repository.save(&c) {
                    if op == "importImage" {
                        let _ = self.repository.cleanup(&c.draft_images);
                    }
                    return Err(e.to_string());
                }
                s.view.conversations[ci] = c;
                self.repository
                    .cleanup(&cleanup)
                    .map_err(|e| e.to_string())?;
            }
            _ => return Err("Unknown lorca operation".into()),
        }
        self.publish(&mut s);
        Ok(json!(s.view))
    }
}

fn export_markdown(chat: &Conversation) -> String {
    let mut output = format!(
        "# {}\n\nLorca conversation export (ID: {})\n\n",
        chat.title, chat.id
    );
    for (index, message) in chat.messages.iter().enumerate() {
        output.push_str(&format!(
            "## {}. {}\n\n",
            index + 1,
            if message.role == "user" {
                "User"
            } else {
                "Lorca"
            }
        ));
        output.push_str(&message.content);
        output.push_str("\n\n");
        if !message.attachments.is_empty() {
            output.push_str("Attachments (stored separately in the local Lorca library):\n");
            for file in &message.attachments {
                output.push_str(&format!("- {} ({})\n", file.name, file.id));
            }
            output.push('\n');
        }
        if let Some(history) = message.extra.get("agentHistory") {
            output.push_str("### Complete agent transcript (JSON)\n\n");
            output.push_str(&serde_json::to_string_pretty(history).unwrap_or_default());
            output.push_str("\n\n");
        }
        if let Some(events) = message.extra.get("agentEvents") {
            output.push_str("### Agent events (JSON)\n\n");
            output.push_str(&serde_json::to_string_pretty(events).unwrap_or_default());
            output.push_str("\n\n");
        }
    }
    output
}
