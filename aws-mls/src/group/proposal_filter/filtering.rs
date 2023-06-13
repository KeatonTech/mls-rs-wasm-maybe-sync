use crate::{
    client::MlsError,
    group::{
        proposal::ReInitProposal,
        proposal_filter::{ProposalBundle, ProposalInfo},
        AddProposal, ProposalType, RemoveProposal, Sender, UpdateProposal,
    },
    key_package::validate_key_package_properties,
    protocol_version::ProtocolVersion,
    time::MlsTime,
    tree_kem::{
        leaf_node_validator::{LeafNodeValidator, ValidationContext},
        node::LeafIndex,
        TreeKemPublic,
    },
    CipherSuiteProvider, ExtensionList,
};

use super::filtering_common::{
    filter_out_invalid_psks, leaf_supports_extensions, ApplyProposalsOutput, ProposalApplier,
};

#[cfg(feature = "external_proposal")]
use crate::extension::ExternalSendersExt;

use alloc::vec::Vec;
use aws_mls_core::{error::IntoAnyError, identity::IdentityProvider, psk::PreSharedKeyStorage};

#[cfg(feature = "custom_proposal")]
use itertools::Itertools;

#[cfg(feature = "external_commit")]
use crate::group::ExternalInit;

#[cfg(feature = "psk")]
use crate::group::proposal::PreSharedKeyProposal;

impl<'a, C, P, CSP> ProposalApplier<'a, C, P, CSP>
where
    C: IdentityProvider,
    P: PreSharedKeyStorage,
    CSP: CipherSuiteProvider,
{
    #[maybe_async::maybe_async]
    pub(super) async fn apply_proposals_from_member(
        &self,
        strategy: FilterStrategy,
        commit_sender: LeafIndex,
        proposals: ProposalBundle,
        commit_time: Option<MlsTime>,
    ) -> Result<ApplyProposalsOutput, MlsError> {
        let proposals = filter_out_invalid_proposers(strategy, proposals)?;

        let mut proposals: ProposalBundle =
            filter_out_update_for_committer(strategy, commit_sender, proposals)?;

        // We ignore the strategy here because the check above ensures all updates are from members
        proposals.update_senders = proposals
            .updates
            .iter()
            .map(leaf_index_of_update_sender)
            .collect::<Result<_, _>>()?;

        let mut proposals = filter_out_removal_of_committer(strategy, commit_sender, proposals)?;

        filter_out_invalid_psks(
            strategy,
            self.cipher_suite_provider,
            &mut proposals,
            self.psk_storage,
        )
        .await?;

        #[cfg(feature = "external_proposal")]
        let proposals = filter_out_invalid_group_extensions(
            strategy,
            proposals,
            self.identity_provider,
            commit_time,
        )
        .await?;

        let proposals = filter_out_extra_group_context_extensions(strategy, proposals)?;
        let proposals = filter_out_invalid_reinit(strategy, proposals, self.protocol_version)?;
        let proposals = filter_out_reinit_if_other_proposals(strategy.is_ignore(), proposals)?;

        #[cfg(feature = "external_commit")]
        let proposals = filter_out_external_init(strategy, proposals)?;

        self.apply_proposal_changes(strategy, proposals, commit_time)
            .await
    }

    #[maybe_async::maybe_async]
    pub(super) async fn apply_proposal_changes(
        &self,
        strategy: FilterStrategy,
        mut proposals: ProposalBundle,
        commit_time: Option<MlsTime>,
    ) -> Result<ApplyProposalsOutput, MlsError> {
        let extensions_proposal_and_capabilities = proposals
            .group_context_extensions_proposal()
            .cloned()
            .and_then(|p| match p.proposal.get_as().map_err(MlsError::from) {
                Ok(capabilities) => Some(Ok((p, capabilities))),
                Err(e) => {
                    if strategy.ignore(p.is_by_reference()) {
                        None
                    } else {
                        Some(Err(e))
                    }
                }
            })
            .transpose()?;

        // If the extensions proposal is ignored, remove it from the list of proposals.
        if extensions_proposal_and_capabilities.is_none() {
            proposals.clear_group_context_extensions();
        }

        match extensions_proposal_and_capabilities {
            Some((group_context_extensions_proposal, new_required_capabilities)) => {
                self.apply_proposals_with_new_capabilities(
                    strategy,
                    proposals,
                    group_context_extensions_proposal,
                    new_required_capabilities,
                    commit_time,
                )
                .await
            }
            None => {
                self.apply_tree_changes(
                    strategy,
                    proposals,
                    self.original_group_extensions,
                    commit_time,
                )
                .await
            }
        }
    }

    #[maybe_async::maybe_async]
    pub(super) async fn apply_tree_changes(
        &self,
        strategy: FilterStrategy,
        proposals: ProposalBundle,
        group_extensions_in_use: &ExtensionList,
        commit_time: Option<MlsTime>,
    ) -> Result<ApplyProposalsOutput, MlsError> {
        let mut applied_proposals = self
            .validate_new_nodes(strategy, proposals, group_extensions_in_use, commit_time)
            .await?;

        let mut new_tree = self.original_tree.clone();

        let added = new_tree
            .batch_edit(
                &mut applied_proposals,
                self.identity_provider,
                self.cipher_suite_provider,
                strategy.is_ignore(),
            )
            .await?;

        Ok(ApplyProposalsOutput {
            applied_proposals,
            new_tree,
            indexes_of_added_kpkgs: added,
            #[cfg(feature = "external_commit")]
            external_init_index: None,
        })
    }

    #[maybe_async::maybe_async]
    async fn validate_new_nodes(
        &self,
        strategy: FilterStrategy,
        mut proposals: ProposalBundle,
        group_extensions_in_use: &ExtensionList,
        commit_time: Option<MlsTime>,
    ) -> Result<ProposalBundle, MlsError> {
        let capabilities = group_extensions_in_use.get_as()?;

        let leaf_node_validator = &LeafNodeValidator::new(
            self.cipher_suite_provider,
            capabilities.as_ref(),
            self.identity_provider,
            Some(group_extensions_in_use),
        );

        for i in (0..proposals.update_proposals().len()).rev() {
            let sender_index = *proposals
                .update_senders
                .get(i)
                .ok_or(MlsError::InternalProposalFilterError)?;

            let res = {
                let leaf = &proposals.update_proposals()[i].proposal.leaf_node;

                let valid = leaf_node_validator
                    .check_if_valid(
                        leaf,
                        ValidationContext::Update((self.group_id, *sender_index, commit_time)),
                    )
                    .await;

                let extensions_are_supported =
                    leaf_supports_extensions(leaf, group_extensions_in_use);

                let old_leaf = self.original_tree.get_leaf_node(sender_index)?;

                let valid_successor = self
                    .identity_provider
                    .valid_successor(&old_leaf.signing_identity, &leaf.signing_identity)
                    .await
                    .map_err(|e| MlsError::IdentityProviderError(e.into_any_error()))
                    .and_then(|valid| valid.then_some(()).ok_or(MlsError::InvalidSuccessor));

                valid.and(extensions_are_supported).and(valid_successor)
            };

            if !apply_strategy(
                strategy,
                proposals.update_proposals()[i].is_by_reference(),
                res,
            )? {
                proposals.remove::<UpdateProposal>(i);
            }
        }

        let mut bad_indices = Vec::new();

        for (i, p) in proposals.by_type::<AddProposal>().enumerate() {
            let valid = leaf_node_validator
                .check_if_valid(
                    &p.proposal.key_package.leaf_node,
                    ValidationContext::Add(commit_time),
                )
                .await;

            let extensions_are_supported = leaf_supports_extensions(
                &p.proposal.key_package.leaf_node,
                group_extensions_in_use,
            );

            let res = valid.and(extensions_are_supported).and(
                validate_key_package_properties(
                    &p.proposal.key_package,
                    self.protocol_version,
                    self.cipher_suite_provider,
                )
                .await,
            );

            if !apply_strategy(strategy, p.is_by_reference(), res)? {
                bad_indices.push(i);
            }
        }

        bad_indices
            .into_iter()
            .rev()
            .for_each(|i| proposals.remove::<AddProposal>(i));

        Ok(proposals)
    }
}

#[derive(Clone, Copy, Debug)]
pub enum FilterStrategy {
    IgnoreByRef,
    IgnoreNone,
}

impl FilterStrategy {
    pub(super) fn ignore(self, by_ref: bool) -> bool {
        match self {
            FilterStrategy::IgnoreByRef => by_ref,
            FilterStrategy::IgnoreNone => false,
        }
    }

    fn is_ignore(self) -> bool {
        match self {
            FilterStrategy::IgnoreByRef => true,
            FilterStrategy::IgnoreNone => false,
        }
    }
}

pub(crate) fn apply_strategy(
    strategy: FilterStrategy,
    by_ref: bool,
    r: Result<(), MlsError>,
) -> Result<bool, MlsError> {
    r.map(|_| true)
        .or_else(|error| strategy.ignore(by_ref).then_some(false).ok_or(error))
}

fn filter_out_update_for_committer(
    strategy: FilterStrategy,
    commit_sender: LeafIndex,
    mut proposals: ProposalBundle,
) -> Result<ProposalBundle, MlsError> {
    proposals.retain_by_type::<UpdateProposal, _, _>(|p| {
        apply_strategy(
            strategy,
            p.is_by_reference(),
            (p.sender != Sender::Member(*commit_sender))
                .then_some(())
                .ok_or(MlsError::InvalidCommitSelfUpdate),
        )
    })?;
    Ok(proposals)
}

fn filter_out_removal_of_committer(
    strategy: FilterStrategy,
    commit_sender: LeafIndex,
    mut proposals: ProposalBundle,
) -> Result<ProposalBundle, MlsError> {
    proposals.retain_by_type::<RemoveProposal, _, _>(|p| {
        apply_strategy(
            strategy,
            p.is_by_reference(),
            (p.proposal.to_remove != commit_sender)
                .then_some(())
                .ok_or(MlsError::CommitterSelfRemoval),
        )
    })?;
    Ok(proposals)
}

#[cfg(feature = "external_proposal")]
#[maybe_async::maybe_async]
async fn filter_out_invalid_group_extensions<C>(
    strategy: FilterStrategy,
    mut proposals: ProposalBundle,
    identity_provider: &C,
    commit_time: Option<MlsTime>,
) -> Result<ProposalBundle, MlsError>
where
    C: IdentityProvider,
{
    let mut bad_indices = Vec::new();

    for (i, p) in proposals.by_type::<ExtensionList>().enumerate() {
        let ext = p.proposal.get_as::<ExternalSendersExt>();

        let res = match ext {
            Ok(None) => Ok(()),
            Ok(Some(extension)) => extension
                .verify_all(identity_provider, commit_time, p.proposal())
                .await
                .map_err(|e| MlsError::IdentityProviderError(e.into_any_error())),
            Err(e) => Err(MlsError::from(e)),
        };

        if !apply_strategy(strategy, p.is_by_reference(), res)? {
            bad_indices.push(i);
        }
    }

    bad_indices
        .into_iter()
        .rev()
        .for_each(|i| proposals.remove::<ExtensionList>(i));

    Ok(proposals)
}

fn filter_out_extra_group_context_extensions(
    strategy: FilterStrategy,
    mut proposals: ProposalBundle,
) -> Result<ProposalBundle, MlsError> {
    let mut found = false;

    proposals.retain_by_type::<ExtensionList, _, _>(|p| {
        apply_strategy(
            strategy,
            p.is_by_reference(),
            (!core::mem::replace(&mut found, true))
                .then_some(())
                .ok_or(MlsError::MoreThanOneGroupContextExtensionsProposal),
        )
    })?;

    Ok(proposals)
}

fn filter_out_invalid_reinit(
    strategy: FilterStrategy,
    mut proposals: ProposalBundle,
    protocol_version: ProtocolVersion,
) -> Result<ProposalBundle, MlsError> {
    proposals.retain_by_type::<ReInitProposal, _, _>(|p| {
        apply_strategy(
            strategy,
            p.is_by_reference(),
            (p.proposal.version >= protocol_version)
                .then_some(())
                .ok_or(MlsError::InvalidProtocolVersionInReInit),
        )
    })?;

    Ok(proposals)
}

fn filter_out_reinit_if_other_proposals(
    filter: bool,
    mut proposals: ProposalBundle,
) -> Result<ProposalBundle, MlsError> {
    let has_other_types = proposals.length() > proposals.reinitializations.len();

    if has_other_types {
        let any_by_val = proposals.reinit_proposals().iter().any(|p| p.is_by_value());

        if any_by_val || (!proposals.reinit_proposals().is_empty() && !filter) {
            return Err(MlsError::OtherProposalWithReInit);
        }

        proposals.reinitializations = Vec::new();
    }

    Ok(proposals)
}

#[cfg(feature = "external_commit")]
fn filter_out_external_init(
    strategy: FilterStrategy,
    mut proposals: ProposalBundle,
) -> Result<ProposalBundle, MlsError> {
    proposals.retain_by_type::<ExternalInit, _, _>(|p| {
        apply_strategy(
            strategy,
            p.is_by_reference(),
            Err(MlsError::InvalidProposalTypeForSender),
        )
    })?;

    Ok(proposals)
}

pub(crate) fn proposer_can_propose(
    proposer: Sender,
    proposal_type: ProposalType,
    by_ref: bool,
) -> Result<(), MlsError> {
    let can_propose = match (proposer, by_ref) {
        (Sender::Member(_), false) => matches!(
            proposal_type,
            ProposalType::ADD
                | ProposalType::REMOVE
                | ProposalType::PSK
                | ProposalType::RE_INIT
                | ProposalType::GROUP_CONTEXT_EXTENSIONS
        ),
        (Sender::Member(_), true) => matches!(
            proposal_type,
            ProposalType::ADD
                | ProposalType::UPDATE
                | ProposalType::REMOVE
                | ProposalType::PSK
                | ProposalType::RE_INIT
                | ProposalType::GROUP_CONTEXT_EXTENSIONS
        ),
        #[cfg(feature = "external_proposal")]
        (Sender::External(_), false) => false,
        #[cfg(feature = "external_proposal")]
        (Sender::External(_), true) => matches!(
            proposal_type,
            ProposalType::ADD
                | ProposalType::REMOVE
                | ProposalType::RE_INIT
                | ProposalType::PSK
                | ProposalType::GROUP_CONTEXT_EXTENSIONS
        ),
        #[cfg(feature = "external_commit")]
        (Sender::NewMemberCommit, false) => matches!(
            proposal_type,
            ProposalType::REMOVE | ProposalType::PSK | ProposalType::EXTERNAL_INIT
        ),
        #[cfg(feature = "external_commit")]
        (Sender::NewMemberCommit, true) => false,
        (Sender::NewMemberProposal, false) => false,
        (Sender::NewMemberProposal, true) => matches!(proposal_type, ProposalType::ADD),
    };

    can_propose
        .then_some(())
        .ok_or(MlsError::InvalidProposalTypeForSender)
}

pub(crate) fn filter_out_invalid_proposers(
    strategy: FilterStrategy,
    mut proposals: ProposalBundle,
) -> Result<ProposalBundle, MlsError> {
    for i in (0..proposals.add_proposals().len()).rev() {
        let p = &proposals.add_proposals()[i];
        let res = proposer_can_propose(p.sender, ProposalType::ADD, p.is_by_reference());

        if !apply_strategy(strategy, p.is_by_reference(), res)? {
            proposals.remove::<AddProposal>(i);
        }
    }

    for i in (0..proposals.update_proposals().len()).rev() {
        let p = &proposals.update_proposals()[i];
        let res = proposer_can_propose(p.sender, ProposalType::UPDATE, p.is_by_reference());

        if !apply_strategy(strategy, p.is_by_reference(), res)? {
            proposals.remove::<UpdateProposal>(i);
        }
    }

    for i in (0..proposals.remove_proposals().len()).rev() {
        let p = &proposals.remove_proposals()[i];
        let res = proposer_can_propose(p.sender, ProposalType::REMOVE, p.is_by_reference());

        if !apply_strategy(strategy, p.is_by_reference(), res)? {
            proposals.remove::<RemoveProposal>(i);
        }
    }

    #[cfg(feature = "psk")]
    for i in (0..proposals.psk_proposals().len()).rev() {
        let p = &proposals.psk_proposals()[i];
        let res = proposer_can_propose(p.sender, ProposalType::PSK, p.is_by_reference());

        if !apply_strategy(strategy, p.is_by_reference(), res)? {
            proposals.remove::<PreSharedKeyProposal>(i);
        }
    }

    for i in (0..proposals.reinit_proposals().len()).rev() {
        let p = &proposals.reinit_proposals()[i];
        let res = proposer_can_propose(p.sender, ProposalType::RE_INIT, p.is_by_reference());

        if !apply_strategy(strategy, p.is_by_reference(), res)? {
            proposals.remove::<ReInitProposal>(i);
        }
    }

    #[cfg(feature = "external_commit")]
    for i in (0..proposals.external_init_proposals().len()).rev() {
        let p = &proposals.external_init_proposals()[i];
        let res = proposer_can_propose(p.sender, ProposalType::EXTERNAL_INIT, p.is_by_reference());

        if !apply_strategy(strategy, p.is_by_reference(), res)? {
            proposals.remove::<ExternalInit>(i);
        }
    }

    for i in (0..proposals.group_context_ext_proposals().len()).rev() {
        let p = &proposals.group_context_ext_proposals()[i];
        let gce_type = ProposalType::GROUP_CONTEXT_EXTENSIONS;
        let res = proposer_can_propose(p.sender, gce_type, p.is_by_reference());

        if !apply_strategy(strategy, p.is_by_reference(), res)? {
            proposals.remove::<ExtensionList>(i);
        }
    }

    Ok(proposals)
}

fn leaf_index_of_update_sender(p: &ProposalInfo<UpdateProposal>) -> Result<LeafIndex, MlsError> {
    match p.sender {
        Sender::Member(i) => Ok(LeafIndex(i)),
        _ => Err(MlsError::InvalidProposalTypeForSender),
    }
}

#[cfg(feature = "custom_proposal")]
pub(super) fn filter_out_unsupported_custom_proposals(
    proposals: &mut ProposalBundle,
    tree: &TreeKemPublic,
    strategy: FilterStrategy,
) -> Result<(), MlsError> {
    let supported_types = proposals
        .custom_proposal_types()
        .filter(|t| tree.can_support_proposal(*t))
        .collect_vec();

    proposals.retain_custom(|p| {
        let proposal_type = p.proposal.proposal_type();

        apply_strategy(
            strategy,
            p.is_by_reference(),
            supported_types
                .contains(&proposal_type)
                .then_some(())
                .ok_or(MlsError::UnsupportedCustomProposal(proposal_type)),
        )
    })
}
