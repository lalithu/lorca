use super::*;

impl Core {
    pub(super) fn send(self: &Arc<Self>, session: &str, v: &Value) -> Result<Value> {
        let mut s = self.state.lock().unwrap();
        let si = Self::session_index(&s, session)?;
        let request_id =
            crate::storage::valid_id(text(v, "requestId")?).map_err(|e| e.to_string())?;
        if let Some(t) = s.view.turns.iter().find(|t| t.id == request_id) {
            return Ok(json!(t));
        }
        let ci = Self::chat_index(&s, text(v, "conversationId")?)?;
        if s.view.conversations[ci]
            .messages
            .iter()
            .any(|m| m.extra.get("requestId").is_some_and(|id| id == &request_id))
        {
            return Err(
                "This prompt was already accepted. Reopen the conversation to see its saved reply."
                    .into(),
            );
        }
        if s.view
            .turns
            .iter()
            .any(|t| t.conversation_id == s.view.conversations[ci].id && t.running)
        {
            return Err("This conversation already has a reply in progress.".into());
        }
        if self.agent_in_chat(&s.view.conversations[ci].id) {
            return Err(
                "This conversation has an active agent task. Stop it before sending a reply."
                    .into(),
            );
        }
        let mut chat = s.view.conversations[ci].clone();
        let content = v["text"].as_str().unwrap_or(&chat.draft).trim().to_string();
        if content.is_empty() && chat.draft_images.is_empty() {
            return Err("Write a message or attach an image.".into());
        }
        let mut message = Message::new("user", content);
        message.attachments = std::mem::take(&mut chat.draft_images);
        if chat.title == "New chat" && chat.messages.is_empty() {
            chat.title = if message.content.is_empty() {
                format!("Image: {}", message.attachments[0].name)
            } else {
                message.content.chars().take(60).collect()
            };
        }
        message.extra.insert("requestId".into(), json!(request_id));
        chat.messages.push(message);
        chat.draft.clear();
        chat.updated_at = now();
        self.repository.save(&chat).map_err(|e| e.to_string())?;
        s.view.conversations[ci] = chat.clone();
        let response = Message::new("assistant", String::new());
        let response_id = response.id.clone();
        s.view.conversations[ci].messages.push(response);
        let (cancel, receiver) = watch::channel(false);
        s.cancels.insert(request_id.clone(), cancel);
        let turn = TurnView {
            id: request_id,
            conversation_id: chat.id.clone(),
            session_id: session.into(),
            model: s.view.sessions[si].model.clone(),
            error: None,
            running: true,
            usage: Usage {
                context_used: None,
                context_limit: s.view.sessions[si].usage.context_limit,
                tokens_per_second: None,
                ..Default::default()
            },
        };
        // Retain one completed turn per chat, enough to reconnect without replaying prompts.
        s.view
            .turns
            .retain(|t| t.running || t.conversation_id != chat.id);
        s.view.turns.push(turn.clone());
        s.view.sessions[si].usage.context_used = None;
        s.view.sessions[si].usage.tokens_per_second = None;
        let model = s.view.sessions[si].model.clone();
        let host = s.clients[session].host.clone();
        self.publish(&mut s);
        let core = self.clone();
        let task_turn = turn.clone();
        tokio::spawn(async move {
            core.generate(task_turn, chat, response_id, host, model, receiver)
                .await;
        });
        Ok(json!(turn))
    }
    pub(super) async fn generate(
        self: Arc<Self>,
        turn: TurnView,
        chat: Conversation,
        response: String,
        host: String,
        model: String,
        mut cancel: watch::Receiver<bool>,
    ) {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let operation = async {
            let core = self.clone();
            let sid = turn.session_id.clone();
            let name = model.clone();
            let ready = tokio::spawn(async move { core.prepare(&sid, &name, false).await })
                .await
                .map_err(|e| e.to_string())??;
            let session = ready["sessions"]
                .as_array()
                .and_then(|s| s.iter().find(|s| s["id"] == turn.session_id))
                .ok_or("Session closed")?;
            if session["activeModel"].as_str() != Some(&model) {
                return Err(session["error"]
                    .as_str()
                    .unwrap_or("Model could not be loaded")
                    .to_string());
            }
            if chat.images().next().is_some() {
                let result = self
                    .client
                    .post(format!("{host}/api/show"))
                    .json(&json!({"model":model}))
                    .timeout(Duration::from_secs(5))
                    .send()
                    .await
                    .map_err(|e| e.to_string())?;
                if result.status().is_success() {
                    let value: Value = result.json().await.map_err(|e| e.to_string())?;
                    if value["capabilities"]
                        .as_array()
                        .is_some_and(|c| !c.iter().any(|v| v == "vision"))
                    {
                        return Err(format!(
                            "{model} doesn’t support images. Choose a vision-capable model or start a text-only chat. Your images are saved."
                        ));
                    }
                }
            }
            let mut messages = self.repository.wire_messages(&chat)?;
            let capacity = self
                .context_limit(&host, &model)
                .await
                .unwrap_or(crate::context::DEFAULT_CAPACITY);
            let objective = chat
                .messages
                .iter()
                .find(|m| m.role == "user")
                .map(|m| m.content.as_str())
                .unwrap_or("");
            crate::context::Context::default().fit(
                &mut messages,
                &[],
                capacity,
                objective,
                &json!({}),
            )?;
            provider::stream_reply(
                self.client.clone(),
                &host,
                provider::chat_body(&model, messages),
                &tx,
            )
            .await
        };
        tokio::pin!(operation);
        let mut error = None;
        let mut usage = turn.usage.clone();
        if !*cancel.borrow() {
            loop {
                tokio::select! {
                    _=cancel.changed()=>break,
                    result=&mut operation=>{if let Err(e)=result{error=Some(e);}while let Ok(update)=rx.try_recv(){self.apply_update(&turn,&response,update,&mut usage);}break;},
                    Some(update)=rx.recv()=>self.apply_update(&turn,&response,update,&mut usage),
                }
            }
        }
        let mut s = self.state.lock().unwrap();
        if let Ok(ci) = Self::chat_index(&s, &chat.id) {
            s.view.conversations[ci]
                .messages
                .retain(|m| m.id != response || !m.content.is_empty());
            match self.repository.save(&s.view.conversations[ci]) {
                Ok(()) => {
                    s.dirty.remove(&chat.id);
                }
                Err(e) => s.view.warning = Some(format!("Chat changes couldn’t be saved: {e}")),
            }
        }
        if let Some(t) = s.view.turns.iter_mut().find(|t| t.id == turn.id) {
            t.running = false;
            t.error = error.clone();
            t.usage = usage.clone();
        }
        if let Ok(i) = Self::session_index(&s, &turn.session_id) {
            s.view.sessions[i].error = error;
            s.view.sessions[i].usage = usage;
        }
        s.cancels.remove(&turn.id);
        self.publish(&mut s);
    }
    pub(super) fn apply_update(
        &self,
        turn: &TurnView,
        response: &str,
        update: Update,
        usage: &mut Usage,
    ) {
        let mut s = self.state.lock().unwrap();
        match update {
            Update::Text(text) => {
                if let Ok(i) = Self::session_index(&s, &turn.session_id) {
                    s.view.sessions[i].is_connected = true;
                }
                if let Ok(ci) = Self::chat_index(&s, &turn.conversation_id)
                    && let Some(m) = s.view.conversations[ci]
                        .messages
                        .iter_mut()
                        .find(|m| m.id == response)
                {
                    m.content.push_str(&text);
                    s.dirty.insert(turn.conversation_id.clone());
                }
            }
            Update::Done(value) => *usage = value,
        }
        self.publish(&mut s);
    }
}
