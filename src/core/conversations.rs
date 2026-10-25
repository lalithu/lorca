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
                        if value.len() > 1024 * 1024 {
                            return Err("Draft is too long.".into());
                        }
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
