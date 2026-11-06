use super::*;

impl Core {
    /// Read-only model and hardware advice. This never downloads, swaps or
    /// resizes a loaded model. Inference-specific memory measurements from
    /// llama.cpp take precedence over Lorca's rough disk-weight estimate.
    pub(super) async fn hardware_advice(&self, session: &str, online: bool) -> Result<Value> {
        let hardware = crate::hardware::HardwareSnapshot::detect(&self.repository.root);
        let engine = self.engine(session)?;
        let host = self.host(session)?;
        let endpoint = if engine == provider::Engine::Ollama {
            "/api/tags"
        } else {
            "/models"
        };
        let list: Value = self
            .client
            .get(format!("{host}{endpoint}"))
            .timeout(Duration::from_secs(5))
            .send()
            .await
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?
            .json()
            .await
            .map_err(|e| e.to_string())?;
        let installed = if engine == provider::Engine::Ollama {
            crate::hardware::installed_ollama(&hardware, &list)
        } else {
            crate::hardware::installed_llama_cpp(&list)
        };
        let selected = {
            let state = self.state.lock().unwrap();
            let i = Self::session_index(&state, session)?;
            state.view.sessions[i].model.clone()
        };
        // Router /props?model= can autoload a model. The advisor must only
        // inspect existing catalog/allocation metadata, never trigger that.
        let allocated = if engine == provider::Engine::Ollama {
            self.context_limit(&host, &selected).await
        } else {
            installed
                .as_array()
                .and_then(|models| {
                    models.iter().find(|m| {
                        m["model"]
                            .as_str()
                            .is_some_and(|name| provider::same_model(name, &selected))
                    })
                })
                .and_then(|m| m["context"].as_u64())
        };
        let mut discovery = Vec::new();
        let mut discovery_warning: Option<String> = None;
        if online {
            let url = "https://huggingface.co/api/models?filter=gguf&sort=downloads&direction=-1&limit=12";
            match self
                .client
                .get(url)
                .timeout(Duration::from_secs(8))
                .send()
                .await
            {
                Ok(response) if response.status().is_success() => {
                    match response.json::<Value>().await {
                        Ok(value) => {
                            for item in value.as_array().into_iter().flatten() {
                                if let Some(id) = item["id"].as_str() {
                                    discovery.push(json!({
                                        "id":id,"downloads":item["downloads"],
                                        "url":format!("https://huggingface.co/{id}"),
                                        "hardwareFit":"unverified: inspect the exact GGUF file size, context and tool capabilities before download"
                                    }));
                                }
                            }
                        }
                        Err(e) => discovery_warning = Some(e.to_string()),
                    }
                }
                Ok(response) => {
                    discovery_warning = Some(format!("Hub catalog HTTP {}", response.status()))
                }
                Err(e) => discovery_warning = Some(e.to_string()),
            }
        }
        Ok(json!({
            "hardware":hardware,
            "runtime":if engine == provider::Engine::Ollama {"ollama"} else {"llama.cpp"},
            "installedModels":installed,
            "currentModel":selected,
            "allocatedContext":allocated,
            "hubCandidates":discovery,
            "hubWarning":discovery_warning,
            "note":"These are conservative resource hints, not model-quality rankings or verified context allocations. Manual model selection and current context overrides are preserved. No download or reload was performed."
        }))
    }

    pub(super) async fn refresh(&self, session: &str) -> Result<Value> {
        use provider::Engine::{LlamaCpp, Ollama};
        let auto = self
            .state
            .lock()
            .unwrap()
            .clients
            .get(session)
            .ok_or("Session expired; reconnect to lorca.")?
            .auto;
        if auto {
            let routes = [
                ModelRoute {
                    provider: Ollama,
                    host: Ollama.default_host().into(),
                },
                ModelRoute {
                    provider: LlamaCpp,
                    host: LlamaCpp.default_host().into(),
                },
                ModelRoute {
                    provider: LlamaCpp,
                    host: "http://127.0.0.1:8080".into(),
                },
            ];
            return self.refresh_local_catalogs(session, &routes).await;
        }
        let route = self.model_route(session)?;
        let outcome = provider::model_list(&self.client, route.provider, &route.host).await;
        let mut s = self.state.lock().unwrap();
        let i = Self::session_index(&s, session)?;
        match outcome {
            Ok(catalog) => {
                if let Some(loaded) = catalog.loaded {
                    Self::update_allocation(&mut s, i, session, &route, &loaded, true);
                    s.view.sessions[i].active_model = Some(loaded);
                }
                let view = &mut s.view.sessions[i];
                view.models = catalog.models;
                if !view
                    .models
                    .iter()
                    .any(|name| provider::same_model(name, &view.model))
                    && let Some(model) = view.models.first()
                {
                    view.model = model.clone();
                }
                view.is_connected = true;
                view.error = None;
            }
            Err(e) => {
                let view = &mut s.view.sessions[i];
                view.models.clear();
                view.is_connected = false;
                view.error = Some(e);
            }
        }
        self.publish(&mut s);
        Ok(json!(s.view))
    }

    async fn refresh_local_catalogs(&self, session: &str, routes: &[ModelRoute]) -> Result<Value> {
        let catalogs = futures_util::future::join_all(
            routes
                .iter()
                .map(|route| provider::model_list(&self.client, route.provider, &route.host)),
        )
        .await;
        let reachable = catalogs.iter().any(|catalog| catalog.is_ok());
        let mut choices = vec![];
        for (route, catalog) in routes.iter().zip(catalogs) {
            if let Ok(catalog) = catalog {
                for model in catalog.models {
                    choices.push(ModelChoice {
                        route: route.clone(),
                        model,
                        managed: catalog.managed,
                    });
                }
            }
        }
        let mut s = self.state.lock().unwrap();
        let i = Self::session_index(&s, session)?;
        let busy = self.model_route_busy(&s, session);
        let saved_preference = s.clients[session].preferred.clone();
        let view = &mut s.view.sessions[i];
        view.models = choices.iter().map(|choice| choice.model.clone()).collect();
        view.model_choices = choices;
        let preferred = view.model_choice.as_ref().or(saved_preference.as_ref());
        let exact = preferred.and_then(|wanted| {
            view.model_choices
                .iter()
                .find(|choice| choice.route == wanted.route && choice.model == wanted.model)
        });
        let next = if busy {
            exact
        } else {
            exact
                .or_else(|| {
                    view.model_choices
                        .iter()
                        .find(|choice| provider::same_model(&choice.model, &view.model))
                })
                .or_else(|| view.model_choices.first())
        }
        .cloned();
        view.is_connected = next.is_some();
        if !busy {
            if let Some(choice) = next {
                Self::apply_model_choice(&mut s, i, session, choice);
            } else {
                view.model_choice = None;
                view.active_model = None;
                view.allocated_context = None;
                view.usage = Usage::default();
            }
            s.view.sessions[i].error = if s.view.sessions[i].is_connected {
                None
            } else if reachable {
                Some("A local server is running, but it has no models available. Add a model in Ollama or llama.cpp, then refresh.".into())
            } else {
                Some("No local model server found. Start Ollama or llama.cpp, then refresh.".into())
            };
        }
        self.publish(&mut s);
        Ok(json!(s.view))
    }

    fn model_route_busy(&self, s: &State, session: &str) -> bool {
        s.loads.contains_key(session)
            || s.view
                .turns
                .iter()
                .any(|turn| turn.session_id == session && turn.running)
            || self
                .agent
                .snapshot()
                .is_some_and(|task| task.owner == session && task.status.active())
    }

    fn apply_model_choice(s: &mut State, i: usize, session: &str, choice: ModelChoice) {
        let client = s
            .clients
            .get_mut(session)
            .expect("session index has a client");
        let view = &mut s.view.sessions[i];
        if client.engine != choice.route.provider || client.host != choice.route.host {
            view.active_model = None;
        }
        if view.model_choice.as_ref() != Some(&choice) {
            view.allocated_context = None;
            view.usage = Usage::default();
        }
        client.engine = choice.route.provider;
        client.host.clone_from(&choice.route.host);
        view.provider = client.engine.label().into();
        view.model.clone_from(&choice.model);
        view.model_choice = Some(choice);
    }

    pub(super) fn select_model(&self, session: &str, v: &Value) -> Result<Value> {
        let model = text(v, "model")?;
        if model.is_empty() || model.len() > 500 || model.chars().any(char::is_control) {
            return Err("Invalid model name".into());
        }
        let mut s = self.state.lock().unwrap();
        let i = Self::session_index(&s, session)?;
        if self.model_route_busy(&s, session) {
            return Err("Finish or stop the current reply before switching models.".into());
        }
        if s.clients[session].auto {
            let route: ModelRoute = serde_json::from_value(v["route"].clone())
                .map_err(|_| "Select a model from the available list.")?;
            let choice = s.view.sessions[i]
                .model_choices
                .iter()
                .find(|choice| choice.route == route && choice.model == model)
                .cloned()
                .ok_or("That model is no longer available. Refresh models.")?;
            s.clients.get_mut(session).unwrap().preferred = Some(choice.clone());
            Self::apply_model_choice(&mut s, i, session, choice);
            s.view.sessions[i].is_connected = true;
            s.view.sessions[i].error = None;
        } else {
            s.view.sessions[i].model = model.into();
            s.view.sessions[i].allocated_context = None;
            s.view.sessions[i].usage = Usage::default();
        }
        self.publish(&mut s);
        Ok(json!(s.view))
    }

    fn model_route(&self, session: &str) -> Result<ModelRoute> {
        let s = self.state.lock().unwrap();
        let c = s
            .clients
            .get(session)
            .ok_or("Session expired; reconnect to lorca.")?;
        Ok(ModelRoute {
            provider: c.engine,
            host: c.host.clone(),
        })
    }

    fn held_by_other(s: &State, session: &str, model: &str, route: &ModelRoute) -> bool {
        s.clients.iter().any(|(id, client)| {
            id != session
                && client.allocations.get(route).is_some_and(|models| {
                    models.iter().any(|name| provider::same_model(name, model))
                })
        })
    }

    fn update_allocation(
        s: &mut State,
        i: usize,
        session: &str,
        route: &ModelRoute,
        model: &str,
        held: bool,
    ) {
        let allocations = &mut s
            .clients
            .get_mut(session)
            .expect("session index has a client")
            .allocations;
        let models = allocations.entry(route.clone()).or_default();
        models.retain(|name| !provider::same_model(name, model));
        if held {
            models.push(model.into());
        }
        if models.is_empty() {
            allocations.remove(route);
        }
        s.view.sessions[i].models_to_release = allocations.values().flatten().cloned().collect();
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
        let (load_cancel, mut load_rx) = watch::channel(false);
        let (route, previous, managed) = {
            let mut s = self.state.lock().unwrap();
            let i = Self::session_index(&s, session)?;
            let client = &s.clients[session];
            let route = ModelRoute {
                provider: client.engine,
                host: client.host.clone(),
            };
            let view = &s.view.sessions[i];
            let managed = view
                .model_choice
                .as_ref()
                .is_none_or(|choice| choice.managed);
            if view
                .active_model
                .as_deref()
                .is_some_and(|m| provider::same_model(m, model))
            {
                s.view.sessions[i].error = None;
                self.publish(&mut s);
                return Ok(json!(s.view));
            }
            let previous = view.active_model.clone();
            s.loads.insert(session.into(), load_cancel);
            s.view.sessions[i].loading_model = Some(model.into());
            s.view.sessions[i].error = None;
            if managed {
                Self::update_allocation(&mut s, i, session, &route, model, true);
            }
            self.publish(&mut s);
            (route, previous, managed)
        };
        let host = &route.host;
        let engine = route.provider;
        let mut released = false;
        let operation = async {
            if managed
                && unload_previous
                && let Some(old) = &previous
            {
                let shared = {
                    let s = self.state.lock().unwrap();
                    Self::held_by_other(&s, session, old, &route)
                };
                if !shared {
                    provider::manage(&self.client, engine, host, old, false).await?;
                    released = true;
                    let mut s = self.state.lock().unwrap();
                    let i = Self::session_index(&s, session)?;
                    Self::update_allocation(&mut s, i, session, &route, old, false);
                    s.view.sessions[i].active_model = None;
                }
            }
            // Another lorca client may already hold this exact allocation.
            let shared = {
                let s = self.state.lock().unwrap();
                s.view.sessions.iter().any(|v| {
                    v.id != session
                        && s.clients
                            .get(&v.id)
                            .is_some_and(|c| c.host == *host && c.engine == engine)
                        && v.active_model
                            .as_deref()
                            .is_some_and(|m| provider::same_model(m, model))
                })
            };
            if managed && !shared {
                provider::manage(&self.client, engine, host, model, true).await?;
            }
            Ok::<_, String>(())
        };
        let result = tokio::select! { result=operation=>result, _=load_rx.changed()=>Err("Model loading cancelled.".into()) };
        if result.is_err()
            && !matches!(&result,Err(message) if message=="Model loading cancelled.")
            && released
            && let Some(old) = &previous
        {
            let _ = provider::manage(&self.client, engine, host, old, true)
                .await
                .map(|()| {
                    let mut s = self.state.lock().unwrap();
                    if let Ok(i) = Self::session_index(&s, session) {
                        s.view.sessions[i].active_model = Some(old.clone());
                        Self::update_allocation(&mut s, i, session, &route, old, true);
                    }
                });
        }
        let limit = if result.is_ok() {
            self.context_limit_for(engine, host, model).await
        } else {
            None
        };
        let mut s = self.state.lock().unwrap();
        s.loads.remove(session);
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
                s.view.sessions[i].allocated_context = limit;
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
    pub(super) async fn context_limit_for(
        &self,
        engine: provider::Engine,
        host: &str,
        model: &str,
    ) -> Option<u64> {
        if engine == provider::Engine::Ollama {
            return self.context_limit(host, model).await;
        }
        let mut url = reqwest::Url::parse(&format!("{host}/props")).ok()?;
        url.query_pairs_mut().append_pair("model", model);
        let value: Value = self
            .client
            .get(url)
            .timeout(Duration::from_secs(2))
            .send()
            .await
            .ok()?
            .json()
            .await
            .ok()?;
        if let Some(limit) = value["default_generation_settings"]["n_ctx"]
            .as_u64()
            .filter(|n| *n > 0)
        {
            return Some(limit);
        }
        // Router /props describes the router itself (n_ctx = 0). Read the selected
        // model's actual allocation, never its larger training context_length.
        let models: Value = self
            .client
            .get(format!("{host}/models"))
            .timeout(Duration::from_secs(2))
            .send()
            .await
            .ok()?
            .error_for_status()
            .ok()?
            .json()
            .await
            .ok()?;
        models["data"].as_array()?.iter().find(|m| {
            m["id"]
                .as_str()
                .is_some_and(|id| provider::same_model(id, model))
        })?["meta"]["n_ctx"]
            .as_u64()
            .filter(|n| *n > 0)
    }
    pub(super) async fn release(&self, session: &str, model: &str) -> Result<()> {
        let route = self.model_route(session)?;
        self.release_from(session, model, &route).await
    }

    async fn release_from(&self, session: &str, model: &str, route: &ModelRoute) -> Result<()> {
        let _guard = self.lifecycle.lock().await;
        {
            let s = self.state.lock().unwrap();
            let i = Self::session_index(&s, session)?;
            if s.view.sessions[i]
                .model_choice
                .as_ref()
                .is_some_and(|choice| choice.route == *route && !choice.managed)
            {
                return Err("This model is managed by its llama.cpp server.".into());
            }
            if Self::held_by_other(&s, session, model, route) {
                return Err(format!(
                    "{model} is still used by another lorca session; kept loaded."
                ));
            }
        }
        provider::manage(&self.client, route.provider, &route.host, model, false).await?;
        let mut s = self.state.lock().unwrap();
        let i = Self::session_index(&s, session)?;
        Self::update_allocation(&mut s, i, session, route, model, false);
        if s.clients[session].engine == route.provider
            && s.clients[session].host == route.host
            && s.view.sessions[i]
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
        self.agent.cancel_owner(session);
        let names = {
            let mut s = self.state.lock().unwrap();
            Self::session_index(&s, session)?;
            Self::cancel_session(&mut s, session);
            if let Some(cancel) = s.loads.get(session) {
                cancel.send_replace(true);
            }
            s.clients[session].allocations.clone()
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
            // Another client's model preparation can hold the lifecycle lock.
            // Give all retirements one shared budget, then release this session
            // even when the provider or another allocation is unresponsive.
            let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
            for (route, models) in names {
                for model in models {
                    if !matches!(
                        tokio::time::timeout_at(
                            deadline,
                            self.release_from(session, &model, &route)
                        )
                        .await,
                        Ok(Ok(()))
                    ) {
                        failed.push(model);
                    }
                }
            }
        }
        for _ in 0..200 {
            if !self.state.lock().unwrap().loads.contains_key(session) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        self.maintenance();
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

#[cfg(test)]
mod tests;
