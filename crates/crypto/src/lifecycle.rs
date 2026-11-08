use super::*;
use croaker_protocol::lifecycle::{LifecycleAction, LifecycleContext};
impl Device {
    fn check_lifecycle(
        &self,
        context: &LifecycleContext,
        authority: &DeviceBinding,
        target: &DeviceBinding,
    ) -> Result<()> {
        context.validate().map_err(|_| CryptoError::InvalidInput)?;
        if context.expected_owner != authority.device_id
            || context.action.target() != target.device_id
            || authority.tenant_id != self.binding.tenant_id
            || target.tenant_id != self.binding.tenant_id
        {
            return Err(CryptoError::IdentityMismatch);
        }
        let mut expected = context.expected_devices.clone();
        expected.sort();
        if self.conversation_devices(context.conversation_id)? != expected {
            return Err(CryptoError::IdentityMismatch);
        }
        let group = self
            .groups
            .get(&context.conversation_id)
            .ok_or(CryptoError::UnknownConversation)?;
        for binding in [authority, target] {
            if !group.members().any(|m| {
                m.credential == binding.credential().credential
                    && m.signature_key == binding.signature_key
            }) {
                return Err(CryptoError::IdentityMismatch);
            }
        }
        Ok(())
    }
    /// Create a context-bound commit; caller must persist state with exact ciphertext.
    pub fn commit_lifecycle(
        &mut self,
        context: &LifecycleContext,
        target: &DeviceBinding,
    ) -> Result<Vec<u8>> {
        self.check_lifecycle(context, &self.binding.clone(), target)?;
        let group = self
            .groups
            .get_mut(&context.conversation_id)
            .ok_or(CryptoError::UnknownConversation)?;
        checked(
            group.set_configuration(self.provider.storage(), config().join_config()),
            "configure delayed application retention",
        )?;
        if group.pending_proposals().next().is_some() {
            return Err(CryptoError::InvalidInput);
        }
        group.set_aad(context.aad());
        let commit = match context.action {
            LifecycleAction::Transfer { .. } => checked(
                group.self_update(&self.provider, &self.signer, LeafNodeParameters::default()),
                "transfer ownership",
            )?
            .commit()
            .clone(),
            LifecycleAction::Remove { target } => {
                let index = group
                    .members()
                    .find(|m| {
                        m.credential
                            == BasicCredential::new(identity(self.binding.tenant_id, target)).into()
                    })
                    .map(|m| m.index)
                    .ok_or(CryptoError::IdentityMismatch)?;
                checked(
                    group.remove_members(&self.provider, &self.signer, &[index]),
                    "remove group member",
                )?
                .0
            }
        };
        let bytes = checked(commit.tls_serialize_detached(), "serialize lifecycle")?;
        checked(
            group.merge_pending_commit(&self.provider),
            "merge lifecycle",
        )?;
        Ok(bytes)
    }
    pub fn apply_lifecycle(
        &mut self,
        context: &LifecycleContext,
        ciphertext: &[u8],
        authority: &DeviceBinding,
        target: &DeviceBinding,
    ) -> Result<()> {
        self.check_lifecycle(context, authority, target)?;
        if ciphertext.len() > MAX_CIPHERTEXT_BYTES {
            return Err(CryptoError::InvalidInput);
        }
        let message = checked(
            MlsMessageIn::tls_deserialize_exact(ciphertext),
            "parse lifecycle",
        )?;
        let message = checked(
            message.try_into_protocol_message(),
            "validate lifecycle type",
        )?;
        let group = self
            .groups
            .get_mut(&context.conversation_id)
            .ok_or(CryptoError::UnknownConversation)?;
        checked(
            group.set_configuration(self.provider.storage(), config().join_config()),
            "configure delayed application retention",
        )?;
        let owner_index = group
            .members()
            .find(|m| {
                m.credential == authority.credential().credential
                    && m.signature_key == authority.signature_key
            })
            .map(|m| m.index)
            .ok_or(CryptoError::IdentityMismatch)?;
        let target_index = group
            .members()
            .find(|m| {
                m.credential == target.credential().credential
                    && m.signature_key == target.signature_key
            })
            .map(|m| m.index)
            .ok_or(CryptoError::IdentityMismatch)?;
        let processed = checked(
            group.process_message(&self.provider, message),
            "authenticate lifecycle",
        )?;
        if processed.sender() != &Sender::Member(owner_index) || processed.aad() != context.aad() {
            return Err(CryptoError::IdentityMismatch);
        }
        let ProcessedMessageContent::StagedCommitMessage(commit) = processed.into_content() else {
            return Err(CryptoError::InvalidInput);
        };
        let mut proposals = commit.queued_proposals();
        let valid = match context.action {
            LifecycleAction::Transfer { .. } => proposals.next().is_none(),
            LifecycleAction::Remove { .. } => {
                matches!(proposals.next().map(|p|p.proposal()),Some(Proposal::Remove(p)) if p.removed()==target_index)
                    && proposals.next().is_none()
            }
        };
        if !valid
            || commit.update_path_leaf_node().is_some_and(|leaf| {
                leaf.credential() != &authority.credential().credential
                    || leaf.signature_key().as_slice() != authority.signature_key
            })
        {
            return Err(CryptoError::IdentityMismatch);
        }
        drop(proposals);
        checked(
            group.merge_staged_commit(&self.provider, *commit),
            "merge approved lifecycle",
        )?;
        Ok(())
    }
}
