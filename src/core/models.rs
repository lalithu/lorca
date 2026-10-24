use super::*;

impl Core {
    pub(super) async fn refresh(&self, session: &str) -> Result<Value> {
        let host = self.host(session)?;
        let outcome = async {
            let value: Value = self
                .client
                .get(format!("{host}/api/tags"))
                .timeout(Duration::from_secs(5))
                .send()
                .await
                .map_err(
                    |_| "Ollama isn’t available. Start it with ‘ollama serve’, then reconnect.",
                )?
                .error_for_status()
                .map_err(|e| e.to_string())?
                .json()
                .await
                .map_err(|_| "Invalid model list")?;
            let mut models = value["models"]
                .as_array()
                .ok_or("Invalid model list")?
                .iter()
                .filter_map(|m| m["name"].as_str().map(String::from))
                .collect::<Vec<_>>();
            models.sort();
            Ok::<_, String>(models)
        }
        .await;
        let mut s = self.state.lock().unwrap();
        let i = Self::session_index(&s, session)?;
        match outcome {
            Ok(models) => {
                s.view.sessions[i].models = models;
                s.view.sessions[i].is_connected = true;
                s.view.sessions[i].error = None;
            }
            Err(e) => {
                s.view.sessions[i].is_connected = false;
                s.view.sessions[i].error = Some(e);
            }
        }
        self.publish(&mut s);
        Ok(json!(s.view))
    }
    pub(super) fn held_by_other(s: &State, session: &str, model: &str, host: &str) -> bool {
        s.view.sessions.iter().any(|v| {
            v.id != session
                && s.clients.get(&v.id).is_some_and(|c| c.host == host)
                && v.models_to_release
                    .iter()
                    .any(|m| provider::same_model(m, model))
        })
    }
    pub(super) async fn prepare(
        &self,
        session: &str,
        model: &str,
        unload_previous: bool,
    ) -> Result<Value> {
        if model.is_empty() || model.len() > 500 || model.chars().any(char::is_control) {
            return Err("Invalid model name".into());
        }
        let _guard = self.lifecycle.lock().await;
        let host = self.host(session)?;
        let (load_cancel, mut load_rx) = watch::channel(false);
        let previous = {
            let mut s = self.state.lock().unwrap();
            let i = Self::session_index(&s, session)?;
            if s.view.sessions[i]
                .active_model
                .as_deref()
                .is_some_and(|m| provider::same_model(m, model))
            {
                s.view.sessions[i].error = None;
                self.publish(&mut s);
                return Ok(json!(s.view));
            }
            s.loads.insert(session.into(), load_cancel);
            s.view.sessions[i].loading_model = Some(model.into());
            s.view.sessions[i].error = None;
            let previous = s.view.sessions[i].active_model.clone();
            if !s.view.sessions[i]
                .models_to_release
                .iter()
                .any(|m| provider::same_model(m, model))
            {
                s.view.sessions[i].models_to_release.push(model.into());
            }
            self.publish(&mut s);
            previous
        };
        let mut released = false;
        let operation = async {
            if unload_previous && let Some(old) = &previous {
                let shared = {
                    let s = self.state.lock().unwrap();
                    Self::held_by_other(&s, session, old, &host)
                };
                if !shared {
                    provider::manage_model(&self.client, &host, old, false).await?;
                    released = true;
                    let mut s = self.state.lock().unwrap();
                    let i = Self::session_index(&s, session)?;
                    s.view.sessions[i]
                        .models_to_release
                        .retain(|m| !provider::same_model(m, old));
                    s.view.sessions[i].active_model = None;
                }
            }
            // Another lorca client may already hold this exact allocation.
            let shared = {
                let s = self.state.lock().unwrap();
                s.view.sessions.iter().any(|v| {
                    v.id != session
                        && s.clients.get(&v.id).is_some_and(|c| c.host == host)
                        && v.active_model
                            .as_deref()
                            .is_some_and(|m| provider::same_model(m, model))
                })
            };
            if !shared {
                provider::manage_model(&self.client, &host, model, true).await?;
            }
            Ok::<_, String>(())
        };
        let result = tokio::select! { result=operation=>result, _=load_rx.changed()=>Err("Model loading cancelled.".into()) };
        self.state.lock().unwrap().loads.remove(session);
        if result.is_err()
            && !matches!(&result,Err(message) if message=="Model loading cancelled.")
            && released
            && let Some(old) = &previous
        {
            let _ = provider::manage_model(&self.client, &host, old, true)
                .await
                .map(|()| {
                    let mut s = self.state.lock().unwrap();
                    if let Ok(i) = Self::session_index(&s, session) {
                        s.view.sessions[i].active_model = Some(old.clone());
                        s.view.sessions[i].models_to_release.push(old.clone());
                    }
                });
        }
        let limit = if result.is_ok() {
            self.context_limit(&host, model).await
        } else {
            None
        };
        let mut s = self.state.lock().unwrap();
        let i = Self::session_index(&s, session)?;
        s.view.sessions[i].loading_model = None;
        match result {
            Ok(()) => {
                s.view.sessions[i].active_model = Some(model.into());
                s.view.sessions[i].model = model.into();
                s.view.sessions[i].is_connected = true;
                let selected = s.view.sessions[i].selected_id.clone();
                let context_used = s
                    .view
                    .conversations
                    .iter()
                    .find(|c| c.id == selected)
                    .and_then(|c| c.messages.is_empty().then_some(0));
                s.view.sessions[i].usage = Usage {
                    context_used,
                    context_limit: limit,
                    ..Default::default()
                };
            }
            Err(e) => s.view.sessions[i].error = Some(e),
        }
        self.publish(&mut s);
        Ok(json!(s.view))
    }
    pub(super) async fn context_limit(&self, host: &str, model: &str) -> Option<u64> {
        let v: Value = self
            .client
            .get(format!("{host}/api/ps"))
            .timeout(Duration::from_secs(1))
            .send()
            .await
            .ok()?
            .json()
            .await
            .ok()?;
        v["models"].as_array()?.iter().find(|m| {
            m["name"]
                .as_str()
                .or_else(|| m["model"].as_str())
                .is_some_and(|n| provider::same_model(n, model))
        })?["context_length"]
            .as_u64()
            .filter(|v| *v > 0)
    }
    pub(super) async fn release(&self, session: &str, model: &str) -> Result<()> {
        let _guard = self.lifecycle.lock().await;
        let host = self.host(session)?;
        {
            let s = self.state.lock().unwrap();
            if Self::held_by_other(&s, session, model, &host) {
                return Err(format!(
                    "{model} is still used by another lorca session; kept loaded."
                ));
            }
        }
        provider::manage_model(&self.client, &host, model, false).await?;
        let mut s = self.state.lock().unwrap();
        let i = Self::session_index(&s, session)?;
        s.view.sessions[i]
            .models_to_release
            .retain(|m| !provider::same_model(m, model));
        if s.view.sessions[i]
            .active_model
            .as_deref()
            .is_some_and(|m| provider::same_model(m, model))
        {
            s.view.sessions[i].active_model = None;
        }
        self.publish(&mut s);
        Ok(())
    }
    pub(super) async fn close(&self, session: &str, unload: bool) -> Result<Value> {
        let names = {
            let mut s = self.state.lock().unwrap();
            let i = Self::session_index(&s, session)?;
            Self::cancel_session(&mut s, session);
            if let Some(cancel) = s.loads.get(session) {
                cancel.send_replace(true);
            }
            s.view.sessions[i].models_to_release.clone()
        };
        for _ in 0..200 {
            if self
                .snapshot()
                .turns
                .iter()
                .all(|t| t.session_id != session || !t.running)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let mut failed = vec![];
        if unload {
            for model in names {
                if self.release(session, &model).await.is_err() {
                    failed.push(model);
                }
            }
        }
        for _ in 0..200 {
            if !self.state.lock().unwrap().loads.contains_key(session) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let mut s = self.state.lock().unwrap();
        s.clients.remove(session);
        s.view.sessions.retain(|v| v.id != session);
        self.publish(&mut s);
        Ok(json!({"failures":failed}))
    }
    pub(super) fn cancel_session(s: &mut State, session: &str) {
        if let Some(cancel) = s.loads.get(session) {
            cancel.send_replace(true);
        }
        for t in s
            .view
            .turns
            .iter()
            .filter(|t| t.session_id == session && t.running)
        {
            if let Some(cancel) = s.cancels.get(&t.id) {
                cancel.send_replace(true);
            }
        }
    }
}
