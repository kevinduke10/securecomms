use super::*;
impl DeviceRuntime {
    pub(super) async fn load_login_config(&mut self) -> Result<oauth::LoginConfig> {
        self.ensure_healthy()?;
        let relay = &self
            .connection
            .as_ref()
            .context("Configure a relay first.")?
            .relay;
        let response = self
            .http
            .get(format!("{relay}/v1/login/config"))
            .timeout(std::time::Duration::from_secs(15))
            .send()
            .await?;
        ensure!(
            response.status().is_success() && response.content_length().is_none_or(|n| n < 8192),
            "Could not load sign-in configuration."
        );
        let bytes = bounded_response(response, 8192).await?;
        Ok(serde_json::from_slice(&bytes)?)
    }
    pub(super) async fn begin_login(&mut self) -> Result<Value> {
        let config = self.load_login_config().await?;
        self.login = None;
        let pending = oauth::begin(config.clone()).await?;
        let url = pending.authorize_url.clone();
        self.login = Some(pending);
        self.login_config = Some(config);
        Ok(json!({"authorize_url":url}))
    }
    pub(super) async fn accept_access(&mut self, access: Zeroizing<String>) -> Result<()> {
        self.ensure_healthy()?;
        self.access = Some(access);
        self.account = None;
        let account = match self.remote("GET", "/v1/account", None, true).await {
            Ok(account) => account,
            Err(error) => {
                self.access = None;
                return Err(error);
            }
        };
        if let Some(profile) = &self.private
            && account["id"].as_str() != Some(&profile.account_id)
        {
            self.access = None;
            bail!("Sign in with this device's original account.");
        }
        if let Some(profile) = &mut self.private {
            profile.locked = false;
            self.save()?;
        }
        self.account = Some(account);
        self.locked = false;
        Ok(())
    }
    pub(super) async fn session(&mut self) -> Result<Value> {
        if let Some(login) = &mut self.login
            && let Some(tokens) = login.try_code().await?
        {
            self.login = None;
            self.accept_access(tokens.access_token).await?;
        }
        if self.access.is_some() && self.private.as_ref().is_some_and(|p| !p.approved) {
            let account = self.remote("GET", "/v1/account", None, true).await?;
            let binding = &self.profile()?.binding;
            let approved = account["devices"].as_array().is_some_and(|devices| {
                devices.iter().any(|d| {
                    d["revoked"] != true
                        && serde_json::from_value::<DeviceBinding>(d["binding"].clone())
                            .is_ok_and(|b| b == *binding)
                })
            });
            if approved {
                self.private.as_mut().unwrap().approved = true;
                self.save()?;
            }
            self.account = Some(account);
        }
        if !self.locked
            && self.private.as_ref().is_some_and(|p| p.approved)
            && self.account.is_none()
            && self.online
        {
            let id = self.profile()?.account_id.clone();
            self.account = self
                .remote("GET", &format!("/v1/enrollments/{id}"), None, false)
                .await
                .ok();
        }
        let ready = !self.locked && self.private.as_ref().is_some_and(|p| p.approved);
        Ok(
            json!({"authenticated":ready,"auto_login":false,"mode":"cognito","desktop":true,"profile":self.private.as_ref().map(|p|p.binding.device_id),"account":self.account,"signed_in":self.access.is_some(),"local_devices":self.private.as_ref().map(|p|vec![json!({"alias":p.binding.device_id,"device_id":p.binding.device_id,"label":"This device"})]).unwrap_or_default()}),
        )
    }
    pub(super) async fn enroll(&mut self) -> Result<Value> {
        let account = self
            .account
            .as_ref()
            .context("Sign in with your invited account first.")?
            .clone();
        if self.private.is_none() {
            let tenant: TenantId = serde_json::from_value(account["tenant_id"].clone())?;
            let client = if let Some(client) = self.client.take() {
                client
            } else {
                ClientStore::create(
                    self.client_path(),
                    Device::new(tenant, DeviceId::new())?,
                    Zeroizing::new(*self.key),
                )?
            };
            self.client = Some(client);
            let binding = self.client()?.update(|d| Ok(d.binding().clone()))?;
            ensure!(
                binding.tenant_id == tenant,
                "Existing device belongs to another tenant"
            );
            let token = random_token();
            let mut enrollment = EnrollmentInput {
                id: MessageId::new(),
                binding: IdentityBinding {
                    tenant_id: binding.tenant_id,
                    device_id: binding.device_id,
                    signature_key: binding.signature_key.clone(),
                },
                label: if cfg!(target_arch = "wasm32") {
                    "Cirra browser".into()
                } else {
                    "Cirra device".into()
                },
                token_hash: hex::encode(Sha256::digest(token.as_bytes())),
                proof: vec![],
            };
            let account_id = account["id"]
                .as_str()
                .context("Missing account ID")?
                .to_owned();
            enrollment.proof = self
                .client()?
                .update(|d| d.sign_enrollment(&enrollment, &account_id))?;
            self.private = Some(Private {
                relay: self
                    .connection
                    .as_ref()
                    .context("Missing relay configuration")?
                    .relay
                    .clone(),
                binding,
                token,
                account_id,
                name: account["name"].as_str().unwrap_or("My device").to_owned(),
                role: serde_json::from_value(account["role"].clone())?,
                enrollment,
                approved: false,
                locked: false,
                contacts: BTreeMap::new(),
                chats: BTreeMap::new(),
                read_receipts: false,
                approvals: vec![],
                lifecycles: vec![],
                group_decisions: vec![],
            });
            self.save()?;
        } else {
            ensure!(
                account["id"].as_str() == Some(&self.profile()?.account_id),
                "Use this device's original account."
            );
            // Expired requests can be renewed with the same candidate keys/credential.
            let old_id = self.profile()?.enrollment.id.to_string();
            if account["requests"].as_array().is_some_and(|requests| {
                requests
                    .iter()
                    .any(|r| r["id"] == old_id && r["expires_at"].as_u64().unwrap_or(0) <= now())
            }) {
                let mut input = self.profile()?.enrollment.clone();
                input.id = MessageId::new();
                let id = self.profile()?.account_id.clone();
                input.proof = self.client()?.update(|d| d.sign_enrollment(&input, &id))?;
                self.private.as_mut().unwrap().enrollment = input;
                self.save()?;
            }
        }
        let input = serde_json::to_value(&self.profile()?.enrollment)?;
        self.remote("POST", "/v1/account/enrollments", Some(&input), true)
            .await?;
        Ok(
            json!({"fingerprint":self.profile()?.binding.fingerprint(),"account_id":self.profile()?.account_id}),
        )
    }
    pub(super) async fn management(
        &mut self,
        method: &str,
        path: &str,
        body: &Value,
    ) -> Result<Option<Value>> {
        let value = match (method, path) {
            ("GET", "/desktop/contact") => {
                let package = self.client()?.update(|d| d.key_package())?;
                let p = self.profile()?;
                json!({"version":1,"name":p.name,"binding":p.binding,"package":BASE64_STANDARD.encode(package)})
            }
            ("POST", "/desktop/contacts") => {
                let contact: Contact = serde_json::from_value(body["contact"].clone())?;
                let fingerprint = body["fingerprint"]
                    .as_str()
                    .context("Compare the full fingerprint first.")?
                    .split_whitespace()
                    .collect::<String>()
                    .to_lowercase();
                ensure!(
                    contact.binding.fingerprint() == fingerprint
                        && contact.binding.tenant_id == self.profile()?.binding.tenant_id
                        && contact.binding.device_id != self.profile()?.binding.device_id,
                    "The verified fingerprint or tenant does not match."
                );
                ensure!(
                    contact.version == 1
                        && !contact.name.trim().is_empty()
                        && contact.name.len() <= 100
                        && !contact.name.chars().any(char::is_control)
                        && contact.binding.signature_key.len() == 32
                        && contact.package.len() <= MAX_CIPHERTEXT_BYTES.div_ceil(3) * 4
                        && BASE64_STANDARD
                            .decode(&contact.package)
                            .is_ok_and(|p| !p.is_empty() && p.len() <= MAX_CIPHERTEXT_BYTES),
                    "Invalid contact"
                );
                let p = self.private.as_mut().unwrap();
                if let Some(existing) = p.contacts.get(&contact.binding.device_id) {
                    ensure!(
                        existing.binding == contact.binding,
                        "Pinned identity changed. Contact was preserved."
                    );
                }
                p.contacts.insert(contact.binding.device_id, contact);
                self.save()?;
                json!({})
            }
            ("GET", "/desktop/invitations") => json!(
                self.profile()?
                    .chats
                    .values()
                    .filter(|c| c.owner == self.profile().unwrap().binding.device_id)
                    .map(|c| json!({"version":1,"chat":c,"bindings":self.peers(c)}))
                    .collect::<Vec<_>>()
            ),
            ("POST", "/desktop/invitations") => {
                ensure!(
                    body["invitation"]["version"] == 1,
                    "Unsupported invitation version"
                );
                let chat: Chat = serde_json::from_value(body["invitation"]["chat"].clone())?;
                let peers: Vec<DeviceBinding> =
                    serde_json::from_value(body["invitation"]["bindings"].clone())?;
                ensure!(
                    chat.members.contains(&self.profile()?.binding.device_id)
                        && chat.members.contains(&chat.owner)
                        && chat.members.len() >= 2
                        && chat.members.len() <= MAX_CONVERSATION_DEVICES
                        && chat.title.len() <= 80
                        && matches!(chat.kind.as_str(), "direct" | "group"),
                    "Invalid invitation"
                );
                let fingerprint = body["fingerprint"]
                    .as_str()
                    .context("Verify the owner's fingerprint first.")?
                    .split_whitespace()
                    .collect::<String>()
                    .to_lowercase();
                let owner = self
                    .profile()?
                    .contacts
                    .get(&chat.owner)
                    .context("Import and verify the owner contact first.")?;
                ensure!(
                    owner.binding.fingerprint() == fingerprint,
                    "Owner fingerprint mismatch"
                );
                for binding in &peers {
                    let pinned = if binding.device_id == self.profile()?.binding.device_id {
                        &self.profile()?.binding
                    } else {
                        &self
                            .profile()?
                            .contacts
                            .get(&binding.device_id)
                            .context("Verify every participant contact first.")?
                            .binding
                    };
                    ensure!(binding == pinned, "Invitation changed a pinned identity");
                }
                let mut devices = peers.iter().map(|b| b.device_id).collect::<Vec<_>>();
                devices.sort();
                let mut expected = chat.members.clone();
                expected.sort();
                ensure!(devices == expected, "Invitation roster mismatch");
                ensure!(
                    !self.profile()?.chats.contains_key(&chat.id),
                    "Conversation already imported"
                );
                self.private.as_mut().unwrap().chats.insert(chat.id, chat);
                self.save()?;
                json!({})
            }
            ("GET", "/local-auth/devices/inspect") => bail!("Enter an account ID."),
            ("POST", "/local-auth/devices/decision") => {
                let account = body["account_id"].as_str().context("Missing account")?;
                ensure!(
                    account.len() == 64 && account.bytes().all(|b| b.is_ascii_hexdigit()),
                    "Invalid account"
                );
                let id: MessageId = serde_json::from_value(body["id"].clone())?;
                self.remote(
                    "POST",
                    &format!("/v1/enrollments/{account}/{id}/decision"),
                    Some(&json!({"fingerprint":body["fingerprint"],"approve":body["approve"]})),
                    false,
                )
                .await?
            }
            ("POST", "/local-auth/devices/revoke") => {
                let device: DeviceId = serde_json::from_value(body["device_id"].clone())?;
                self.remote(
                    "POST",
                    &format!(
                        "/v1/enrollments/{}/devices/{device}/revoke",
                        self.profile()?.account_id
                    ),
                    Some(&json!({})),
                    false,
                )
                .await?
            }
            _ => {
                if method == "GET"
                    && let Some(account) = path.strip_prefix("/local-auth/devices/inspect?account=")
                {
                    ensure!(
                        account.len() == 64 && account.bytes().all(|b| b.is_ascii_hexdigit()),
                        "Invalid account"
                    );
                    return Ok(Some(
                        self.remote("GET", &format!("/v1/enrollments/{account}"), None, false)
                            .await?,
                    ));
                }
                return Ok(None);
            }
        };
        Ok(Some(value))
    }
    pub(super) fn peers(&self, chat: &Chat) -> Vec<DeviceBinding> {
        let p = self.private.as_ref().unwrap();
        chat.members
            .iter()
            .filter_map(|id| {
                if *id == p.binding.device_id {
                    Some(p.binding.clone())
                } else {
                    p.contacts.get(id).map(|c| c.binding.clone())
                }
            })
            .collect()
    }
}
