#![cfg(test)]

use super::*;

use crate::here;
use crate::{
    conductor::{
        api::{
            AdminInterfaceApi, AdminRequest, AdminResponse, AppInterfaceApi, AppRequest,
            RealAdminInterfaceApi, RealAppInterfaceApi,
        },
        ConductorBuilder,
    },
    core::{
        ribosome::{
            HostContext, HostContextFixturator, NamedInvocation, ZomeCallInvocationFixturator,
        },
        state::{
            cascade::{test_dbs_and_mocks, Cascade},
            metadata::{LinkMetaKey, LinkMetaVal},
            source_chain::SourceChain,
            workspace::WorkspaceError,
        },
        SourceChainError,
    },
    fixt::EntryFixturator,
};
use fixt::prelude::*;
use futures::future::BoxFuture;
use futures::future::FutureExt;
use holo_hash::{Hashable, Hashed, HeaderHash};
use holochain_keystore::Signature;
use holochain_state::{
    buffer::BufferedStore,
    env::{EnvironmentReadRef, EnvironmentWrite, EnvironmentWriteRef, ReadManager, WriteManager},
    error::{DatabaseError, DatabaseResult},
    test_utils::{test_cell_env, test_conductor_env, test_wasm_env, TestEnvironment},
};
use holochain_types::{
    app::{InstallAppDnaPayload, InstallAppPayload},
    composite_hash::{AnyDhtHash, EntryHash},
    dht_op::{DhtOp, DhtOpHashed},
    fixt::{
        AppEntryTypeFixturator, ElementDeleteFixturator, EntryUpdateFixturator, HeaderFixturator,
        LinkAddFixturator, LinkRemoveFixturator, LinkTagFixturator, NewEntryHeaderFixturator,
        SignatureFixturator, ZomeIdFixturator,
    },
    header::{builder, ElementDelete, EntryType, EntryUpdate, LinkAdd, LinkRemove, NewEntryHeader},
    observability,
    test_utils::{fake_agent_pubkey_1, fake_dna_zomes, write_fake_dna_file},
    validate::ValidationStatus,
    Entry, EntryHashed,
};
use holochain_wasm_test_utils::TestWasm;
use holochain_zome_types::HostInput;
use matches::assert_matches;
use std::{convert::TryInto, sync::Arc};
use unwrap_to::unwrap_to;
use uuid::Uuid;

struct TestData {
    signature: Signature,
    original_entry: Entry,
    new_entry: Entry,
    any_header: Header,
    entry_update_header: EntryUpdate,
    entry_update_entry: EntryUpdate,
    original_header_hash: HeaderHash,
    original_entry_hash: EntryHash,
    new_entry_hash: EntryHash,
    original_header: NewEntryHeader,
    entry_delete: ElementDelete,
    link_add: LinkAdd,
    link_remove: LinkRemove,
}

impl TestData {
    #[instrument()]
    async fn new() -> Self {
        // original entry
        let original_entry = fixt!(Entry);
        let original_entry_hash = EntryHashed::with_data(original_entry.clone())
            .await
            .unwrap()
            .into_hash();

        // New entry
        let new_entry = fixt!(Entry);
        let new_entry_hash = EntryHashed::with_data(new_entry.clone())
            .await
            .unwrap()
            .into_hash();

        // Original entry and header for updates
        let mut original_header = fixt!(NewEntryHeader);

        match &mut original_header {
            NewEntryHeader::Create(c) => c.entry_hash = original_entry_hash.clone(),
            NewEntryHeader::Update(u) => u.entry_hash = original_entry_hash.clone(),
        }

        let original_header_hash = HeaderHashed::with_data(original_header.clone().into())
            .await
            .unwrap()
            .into_hash();

        // Header for the new entry
        let mut new_entry_header = fixt!(NewEntryHeader);

        // Update to new entry
        match &mut new_entry_header {
            NewEntryHeader::Create(c) => c.entry_hash = new_entry_hash.clone(),
            NewEntryHeader::Update(u) => u.entry_hash = new_entry_hash.clone(),
        }

        // Entry update for header
        let mut entry_update_header = fixt!(EntryUpdate);
        entry_update_header.entry_hash = new_entry_hash.clone();
        entry_update_header.intended_for = IntendedFor::Header;
        entry_update_header.replaces_address = original_header_hash.clone();

        // Entry update for entry
        let mut entry_update_entry = fixt!(EntryUpdate);
        entry_update_entry.entry_hash = new_entry_hash.clone();
        entry_update_entry.intended_for = IntendedFor::Entry;
        entry_update_entry.replaces_address = original_header_hash.clone();

        // Entry delete
        let mut entry_delete = fixt!(ElementDelete);
        entry_delete.removes_address = original_header_hash.clone();

        // Link add
        let mut link_add = fixt!(LinkAdd);
        link_add.base_address = original_entry_hash.clone();
        link_add.target_address = new_entry_hash.clone();
        link_add.zome_id = fixt!(ZomeId);
        link_add.tag = fixt!(LinkTag);

        let link_add_hash = HeaderHashed::with_data(link_add.clone().into())
            .await
            .unwrap()
            .into_hash();

        // Link remove
        let mut link_remove = fixt!(LinkRemove);
        link_remove.base_address = original_entry_hash.clone();
        link_remove.link_add_address = link_add_hash.clone();

        Self {
            signature: fixt!(Signature),
            original_entry,
            new_entry,
            any_header: fixt!(Header),
            entry_update_header,
            entry_update_entry,
            original_header,
            original_header_hash,
            original_entry_hash,
            entry_delete,
            link_add,
            link_remove,
            new_entry_hash,
        }
    }
}

enum Db {
    Integrated(DhtOp),
    IntegratedEmpty,
    IntQueue(DhtOp),
    CasHeader(Header, Option<Signature>),
    CasEntry(Entry, Option<Header>, Option<Signature>),
    MetaEmpty,
    MetaHeader(Entry, Header),
    MetaActivity(Header),
    MetaUpdate(AnyDhtHash, Header),
    MetaDelete(AnyDhtHash, Header),
    MetaLink(LinkAdd, EntryHash),
    MetaLinkEmpty(LinkAdd),
}

impl Db {
    /// Checks that the database is in a state
    #[instrument(skip(expects, env_ref, dbs))]
    async fn check<'env>(
        expects: Vec<Self>,
        env_ref: &'env EnvironmentReadRef<'env>,
        dbs: &'env impl GetDb,
        here: String,
    ) {
        let reader = env_ref.reader().unwrap();
        let workspace = IntegrateDhtOpsWorkspace::new(&reader, dbs).unwrap();
        for expect in expects {
            match expect {
                Db::Integrated(op) => {
                    let op_hash = DhtOpHashed::with_data(op.clone()).await.into_hash();
                    let (op, basis) =
                        dht_op_to_light_basis(op, &workspace.cas)
                            .await
                            .expect(&format!(
                                "Failed to generate light {} for {}",
                                op_hash, here
                            ));
                    let value = IntegratedDhtOpsValue {
                        validation_status: ValidationStatus::Valid,
                        basis,
                        op,
                    };
                    assert_eq!(
                        workspace.integrated_dht_ops.get(&op_hash).unwrap(),
                        Some(value),
                        "{}",
                        here
                    );
                }
                Db::IntQueue(op) => {
                    let value = IntegrationQueueValue {
                        validation_status: ValidationStatus::Valid,
                        op,
                    };
                    let res = workspace
                        .integration_queue
                        .iter()
                        .unwrap()
                        .filter_map(|(_, v)| if v == value { Ok(Some(v)) } else { Ok(None) })
                        .collect::<Vec<_>>()
                        .unwrap();
                    let exp = [value];
                    assert_eq!(&res[..], &exp[..], "{}", here,);
                }
                Db::CasHeader(header, _) => {
                    let hash = HeaderHashed::with_data(header.clone()).await.unwrap();
                    assert_eq!(
                        workspace
                            .cas
                            .get_header(hash.as_hash())
                            .await
                            .unwrap()
                            .expect(&format!("Header {:?} not in cas for {}", header, here))
                            .header(),
                        &header,
                        "{}",
                        here,
                    );
                }
                Db::CasEntry(entry, _, _) => {
                    let hash = EntryHashed::with_data(entry.clone())
                        .await
                        .unwrap()
                        .into_hash();
                    assert_eq!(
                        workspace
                            .cas
                            .get_entry(&hash)
                            .await
                            .unwrap()
                            .expect(&format!("Entry {:?} not in cas for {}", entry, here))
                            .into_content(),
                        entry,
                        "{}",
                        here,
                    );
                }
                Db::MetaHeader(entry, header) => {
                    let header_hash = HeaderHashed::with_data(header.clone())
                        .await
                        .unwrap()
                        .into_hash();
                    let entry_hash = EntryHashed::with_data(entry.clone())
                        .await
                        .unwrap()
                        .into_hash();
                    let res = workspace
                        .meta
                        .get_headers(entry_hash)
                        .unwrap()
                        .collect::<Vec<_>>()
                        .unwrap();
                    let exp = [header_hash];
                    assert_eq!(&res[..], &exp[..], "{}", here,);
                }
                Db::MetaActivity(header) => {
                    let header_hash = HeaderHashed::with_data(header.clone())
                        .await
                        .unwrap()
                        .into_hash();
                    let res = workspace
                        .meta
                        .get_activity(header.author().clone())
                        .unwrap()
                        .collect::<Vec<_>>()
                        .unwrap();
                    let exp = [header_hash];
                    assert_eq!(&res[..], &exp[..], "{}", here,);
                }
                Db::MetaUpdate(base, header) => {
                    let header_hash = HeaderHashed::with_data(header.clone())
                        .await
                        .unwrap()
                        .into_hash();
                    let res = workspace
                        .meta
                        .get_updates(base)
                        .unwrap()
                        .collect::<Vec<_>>()
                        .unwrap();
                    let exp = [header_hash];
                    assert_eq!(&res[..], &exp[..], "{}", here,);
                }
                Db::MetaDelete(base, header) => {
                    let header_hash = HeaderHashed::with_data(header.clone())
                        .await
                        .unwrap()
                        .into_hash();
                    let res = workspace
                        .meta
                        .get_deletes(base)
                        .unwrap()
                        .collect::<Vec<_>>()
                        .unwrap();
                    let exp = [header_hash];
                    assert_eq!(&res[..], &exp[..], "{}", here,);
                }
                Db::IntegratedEmpty => {
                    assert_eq!(
                        workspace
                            .integrated_dht_ops
                            .iter()
                            .unwrap()
                            .count()
                            .unwrap(),
                        0,
                        "{}",
                        here
                    );
                }
                Db::MetaEmpty => {
                    // TODO: Not currently possible because kvv bufs have no iterator over all keys
                }
                Db::MetaLink(link_add, target_hash) => {
                    let link_add_hash = HeaderHashed::with_data(link_add.clone().into())
                        .await
                        .unwrap()
                        .into_hash();

                    // LinkMetaKey
                    let mut link_meta_keys = Vec::new();
                    link_meta_keys.push(LinkMetaKey::Full(
                        &link_add.base_address,
                        link_add.zome_id,
                        &link_add.tag,
                        &link_add_hash,
                    ));
                    link_meta_keys.push(LinkMetaKey::BaseZomeTag(
                        &link_add.base_address,
                        link_add.zome_id,
                        &link_add.tag,
                    ));
                    link_meta_keys.push(LinkMetaKey::BaseZome(
                        &link_add.base_address,
                        link_add.zome_id,
                    ));
                    link_meta_keys.push(LinkMetaKey::Base(&link_add.base_address));

                    for link_meta_key in link_meta_keys {
                        let res = workspace.meta.get_links(&link_meta_key).unwrap();

                        assert_eq!(res.len(), 1, "{}", here);
                        assert_eq!(res[0].link_add_hash, link_add_hash, "{}", here);
                        assert_eq!(res[0].target, target_hash, "{}", here);
                        assert_eq!(res[0].zome_id, link_add.zome_id, "{}", here);
                        assert_eq!(res[0].tag, link_add.tag, "{}", here);
                    }
                }
                Db::MetaLinkEmpty(link_add) => {
                    let link_add_hash = HeaderHashed::with_data(link_add.clone().into())
                        .await
                        .unwrap()
                        .into_hash();

                    // LinkMetaKey
                    let mut link_meta_keys = Vec::new();
                    link_meta_keys.push(LinkMetaKey::Full(
                        &link_add.base_address,
                        link_add.zome_id,
                        &link_add.tag,
                        &link_add_hash,
                    ));
                    link_meta_keys.push(LinkMetaKey::BaseZomeTag(
                        &link_add.base_address,
                        link_add.zome_id,
                        &link_add.tag,
                    ));
                    link_meta_keys.push(LinkMetaKey::BaseZome(
                        &link_add.base_address,
                        link_add.zome_id,
                    ));
                    link_meta_keys.push(LinkMetaKey::Base(&link_add.base_address));

                    for link_meta_key in link_meta_keys {
                        let res = workspace.meta.get_links(&link_meta_key).unwrap();

                        assert_eq!(res.len(), 0, "{}", here);
                    }
                }
            }
        }
    }

    // Sets the database to a certain state
    #[instrument(skip(pre_state, env_ref, dbs))]
    async fn set<'env>(
        pre_state: Vec<Self>,
        env_ref: &'env EnvironmentWriteRef<'env>,
        dbs: &impl GetDb,
    ) {
        let reader = env_ref.reader().unwrap();
        let mut workspace = IntegrateDhtOpsWorkspace::new(&reader, dbs).unwrap();
        for state in pre_state {
            match state {
                Db::Integrated(_) => {}
                Db::IntQueue(op) => {
                    let op_hash = DhtOpHashed::with_data(op.clone()).await.into_hash();
                    let val = IntegrationQueueValue {
                        validation_status: ValidationStatus::Valid,
                        op,
                    };
                    workspace
                        .integration_queue
                        .put((TimestampKey::now(), op_hash).try_into().unwrap(), val)
                        .unwrap();
                }
                Db::CasHeader(header, signature) => {
                    let header_hash = HeaderHashed::with_data(header.clone()).await.unwrap();
                    debug!(header_hash = %header_hash.as_hash());
                    let signed_header =
                        SignedHeaderHashed::with_presigned(header_hash, signature.unwrap());
                    workspace.cas.put(signed_header, None).unwrap();
                }
                Db::CasEntry(entry, header, signature) => {
                    let header_hash = HeaderHashed::with_data(header.unwrap().clone())
                        .await
                        .unwrap();
                    let entry_hash = EntryHashed::with_data(entry.clone()).await.unwrap();
                    let signed_header =
                        SignedHeaderHashed::with_presigned(header_hash, signature.unwrap());
                    workspace.cas.put(signed_header, Some(entry_hash)).unwrap();
                }
                Db::MetaHeader(_, _) => {}
                Db::MetaActivity(_) => {}
                Db::MetaUpdate(_, _) => {}
                Db::IntegratedEmpty => {}
                Db::MetaEmpty => {}
                Db::MetaDelete(_, _) => {}
                Db::MetaLink(link_add, _) => {
                    workspace.meta.add_link(link_add).await.unwrap();
                }
                Db::MetaLinkEmpty(_) => {}
            }
        }
        // Commit workspace
        env_ref
            .with_commit::<WorkspaceError, _, _>(|writer| {
                workspace.flush_to_txn(writer)?;
                Ok(())
            })
            .unwrap();
    }
}

async fn call_workflow<'env>(
    env_ref: &'env EnvironmentReadRef<'env>,
    dbs: &'env impl GetDb,
    env: EnvironmentWrite,
) {
    let reader = env_ref.reader().unwrap();
    let workspace = IntegrateDhtOpsWorkspace::new(&reader, dbs).unwrap();
    let (mut qt, _rx) = TriggerSender::new();
    integrate_dht_ops_workflow(workspace, env.into(), &mut qt)
        .await
        .unwrap();
}

// Need to clear the data from the previous test
fn clear_dbs<'env>(env_ref: &'env EnvironmentWriteRef<'env>, dbs: &'env impl GetDb) {
    let reader = env_ref.reader().unwrap();
    let mut workspace = IntegrateDhtOpsWorkspace::new(&reader, dbs).unwrap();
    env_ref
        .with_commit::<DatabaseError, _, _>(|writer| {
            workspace.integration_queue.clear_all(writer)?;
            workspace.integrated_dht_ops.clear_all(writer)?;
            workspace.cas.clear_all(writer)?;
            workspace.meta.clear_all(writer)?;
            Ok(())
        })
        .unwrap();
}

// TESTS BEGIN HERE
// The following show an op or ops that you want to test
// with a desired pre-state that you want the database in
// and the expected state of the database after the workflow is run

fn store_element(a: TestData) -> (Vec<Db>, Vec<Db>, &'static str) {
    let entry = match &a.any_header {
        Header::EntryCreate(_) | Header::EntryUpdate(_) => Some(a.original_entry.clone().into()),
        _ => None,
    };
    let op = DhtOp::StoreElement(
        a.signature.clone(),
        a.any_header.clone().into(),
        entry.clone(),
    );
    let pre_state = vec![Db::IntQueue(op.clone())];
    let mut expect = vec![
        Db::Integrated(op.clone()),
        Db::CasHeader(a.any_header.clone().into(), None),
    ];
    if let Some(_) = &entry {
        expect.push(Db::CasEntry(a.original_entry.clone(), None, None));
    }
    (pre_state, expect, "store element")
}

fn store_entry(a: TestData) -> (Vec<Db>, Vec<Db>, &'static str) {
    let op = DhtOp::StoreEntry(
        a.signature.clone(),
        a.original_header.clone(),
        a.original_entry.clone().into(),
    );
    let pre_state = vec![Db::IntQueue(op.clone())];
    let expect = vec![
        Db::Integrated(op.clone()),
        Db::CasHeader(a.original_header.clone().into(), None),
        Db::CasEntry(a.original_entry.clone(), None, None),
        Db::MetaHeader(a.original_entry.clone(), a.original_header.clone().into()),
    ];
    (pre_state, expect, "store entry")
}

fn register_agent_activity(a: TestData) -> (Vec<Db>, Vec<Db>, &'static str) {
    let op = DhtOp::RegisterAgentActivity(a.signature.clone(), a.any_header.clone());
    let pre_state = vec![Db::IntQueue(op.clone())];
    let expect = vec![
        Db::Integrated(op.clone()),
        Db::MetaActivity(a.any_header.clone()),
    ];
    (pre_state, expect, "register agent activity")
}

fn register_replaced_by_for_header(a: TestData) -> (Vec<Db>, Vec<Db>, &'static str) {
    let op = DhtOp::RegisterReplacedBy(
        a.signature.clone(),
        a.entry_update_header.clone(),
        Some(a.new_entry.clone().into()),
    );
    let pre_state = vec![Db::IntQueue(op.clone())];
    let expect = vec![
        Db::Integrated(op.clone()),
        Db::MetaUpdate(
            a.original_header_hash.clone().into(),
            a.entry_update_header.clone().into(),
        ),
    ];
    (pre_state, expect, "register replaced by for header")
}

fn register_replaced_by_for_entry(a: TestData) -> (Vec<Db>, Vec<Db>, &'static str) {
    let op = DhtOp::RegisterReplacedBy(
        a.signature.clone(),
        a.entry_update_entry.clone(),
        Some(a.new_entry.clone().into()),
    );
    let pre_state = vec![
        Db::IntQueue(op.clone()),
        Db::CasHeader(a.original_header.clone().into(), Some(a.signature.clone())),
    ];
    let expect = vec![
        Db::Integrated(op.clone()),
        Db::MetaUpdate(
            a.original_entry_hash.clone().into(),
            a.entry_update_entry.clone().into(),
        ),
    ];
    (pre_state, expect, "register replaced by for entry")
}

// Register replaced by without store entry
fn register_replaced_by_missing_entry(a: TestData) -> (Vec<Db>, Vec<Db>, &'static str) {
    let op = DhtOp::RegisterReplacedBy(
        a.signature.clone(),
        a.entry_update_entry.clone(),
        Some(a.new_entry.clone().into()),
    );
    let pre_state = vec![Db::IntQueue(op.clone())];
    let expect = vec![Db::IntegratedEmpty, Db::IntQueue(op.clone()), Db::MetaEmpty];
    (
        pre_state,
        expect,
        "register replaced by for entry missing entry",
    )
}

fn register_deleted_by(a: TestData) -> (Vec<Db>, Vec<Db>, &'static str) {
    let op = DhtOp::RegisterDeletedEntryHeader(a.signature.clone(), a.entry_delete.clone());
    let pre_state = vec![
        Db::IntQueue(op.clone()),
        Db::CasHeader(a.original_header.clone().into(), Some(a.signature.clone())),
    ];
    let expect = vec![
        Db::Integrated(op.clone()),
        Db::MetaDelete(
            a.original_entry_hash.clone().into(),
            a.entry_delete.clone().into(),
        ),
    ];
    (pre_state, expect, "register deleted by")
}

fn register_deleted_by_missing_entry(a: TestData) -> (Vec<Db>, Vec<Db>, &'static str) {
    let op = DhtOp::RegisterDeletedEntryHeader(a.signature.clone(), a.entry_delete.clone());
    let pre_state = vec![Db::IntQueue(op.clone())];
    let expect = vec![Db::IntegratedEmpty, Db::IntQueue(op.clone()), Db::MetaEmpty];
    (
        pre_state,
        expect,
        "register deleted by for entry missing entry",
    )
}

fn register_deleted_header_by(a: TestData) -> (Vec<Db>, Vec<Db>, &'static str) {
    let op = DhtOp::RegisterDeletedBy(a.signature.clone(), a.entry_delete.clone());
    let pre_state = vec![Db::IntQueue(op.clone())];
    let expect = vec![
        Db::Integrated(op.clone()),
        Db::MetaDelete(
            a.original_header_hash.clone().into(),
            a.entry_delete.clone().into(),
        ),
    ];
    (pre_state, expect, "register deleted header by")
}

fn register_add_link(a: TestData) -> (Vec<Db>, Vec<Db>, &'static str) {
    let op = DhtOp::RegisterAddLink(a.signature.clone(), a.link_add.clone());
    let pre_state = vec![
        Db::IntQueue(op.clone()),
        Db::CasEntry(
            a.original_entry.clone().into(),
            Some(a.original_header.clone().into()),
            Some(a.signature.clone()),
        ),
    ];
    let expect = vec![
        Db::Integrated(op.clone()),
        Db::MetaLink(a.link_add.clone(), a.new_entry_hash.clone().into()),
    ];
    (pre_state, expect, "register link add")
}

fn register_remove_link(a: TestData) -> (Vec<Db>, Vec<Db>, &'static str) {
    let op = DhtOp::RegisterRemoveLink(a.signature.clone(), a.link_remove.clone());
    let pre_state = vec![
        Db::IntQueue(op.clone()),
        Db::CasHeader(a.link_add.clone().into(), Some(a.signature.clone())),
        Db::CasEntry(
            a.original_entry.clone().into(),
            Some(a.original_header.clone().into()),
            Some(a.signature.clone()),
        ),
        Db::MetaLink(a.link_add.clone(), a.new_entry_hash.clone().into()),
    ];
    let expect = vec![
        Db::Integrated(op.clone()),
        Db::MetaLinkEmpty(a.link_add.clone()),
    ];
    (pre_state, expect, "register link remove")
}

// The header isn't stored yet
fn register_remove_link_missing_add_header(a: TestData) -> (Vec<Db>, Vec<Db>, &'static str) {
    let op = DhtOp::RegisterRemoveLink(a.signature.clone(), a.link_remove.clone());
    let pre_state = vec![
        Db::IntQueue(op.clone()),
        Db::CasEntry(
            a.original_entry.clone().into(),
            Some(a.original_header.clone().into()),
            Some(a.signature.clone()),
        ),
    ];
    let expect = vec![Db::IntegratedEmpty, Db::IntQueue(op.clone()), Db::MetaEmpty];
    (
        pre_state,
        expect,
        "register remove link remove missing add header",
    )
}

// Link add is there but metadata is missing
fn register_remove_link_missing_add_metadata(a: TestData) -> (Vec<Db>, Vec<Db>, &'static str) {
    let op = DhtOp::RegisterRemoveLink(a.signature.clone(), a.link_remove.clone());
    let pre_state = vec![
        Db::IntQueue(op.clone()),
        Db::CasHeader(a.link_add.clone().into(), Some(a.signature.clone())),
        Db::CasEntry(
            a.original_entry.clone().into(),
            Some(a.original_header.clone().into()),
            Some(a.signature.clone()),
        ),
    ];
    let expect = vec![Db::IntegratedEmpty, Db::IntQueue(op.clone()), Db::MetaEmpty];
    (
        pre_state,
        expect,
        "register remove link remove missing add metadata",
    )
}

// Link remove when not an author
fn register_remove_link_missing_base(a: TestData) -> (Vec<Db>, Vec<Db>, &'static str) {
    let op = DhtOp::RegisterRemoveLink(a.signature.clone(), a.link_remove.clone());
    let pre_state = vec![Db::IntQueue(op.clone())];
    let expect = vec![Db::IntegratedEmpty, Db::IntQueue(op.clone()), Db::MetaEmpty];
    (
        pre_state,
        expect,
        "register remove link remove missing base",
    )
}

// This runs the above tests
#[tokio::test(threaded_scheduler)]
async fn test_ops_state() {
    observability::test_run().ok();
    let env = test_cell_env();
    let dbs = env.dbs().await;
    let env_ref = env.guard().await;

    let tests = [
        store_element,
        store_entry,
        register_agent_activity,
        register_replaced_by_for_header,
        register_replaced_by_for_entry,
        register_replaced_by_missing_entry,
        register_deleted_by,
        register_deleted_by_missing_entry,
        register_deleted_header_by,
        register_add_link,
        register_remove_link,
        register_remove_link_missing_add_header,
        register_remove_link_missing_add_metadata,
        register_remove_link_missing_base,
    ];

    for t in tests.iter() {
        clear_dbs(&env_ref, &dbs);
        let td = TestData::new().await;
        let (pre_state, expect, name) = t(td);
        Db::set(pre_state, &env_ref, &dbs).await;
        call_workflow(&env_ref, &dbs, env.clone()).await;
        Db::check(expect, &env_ref, &dbs, format!("{}: {}", name, here!(""))).await;
    }
}

// TODO: Actually use the wasm host functions instead of
// implementing these mocks
fn sync_call<'a>(host_context: Arc<HostContext>, base: EntryHash) -> Vec<LinkMetaVal> {
    let call = |workspace: &'a mut InvokeZomeWorkspace| -> BoxFuture<'a, DatabaseResult<Vec<LinkMetaVal>>> {
            async move {
                // TODO: Add link
                // This is a commit though so we can't do that here
                // Get link
                let key = LinkMetaKey::Base(&base);
                let val = workspace.cascade().dht_get_links(&key).await?;
                assert_eq!(val.len(), 1);
                Ok(val)
            }
            .boxed()
        };
    tokio_safe_block_on::tokio_safe_block_forever_on(tokio::task::spawn(async move {
        unsafe { host_context.workspace().apply_mut(call).await }
    }))
    .unwrap()
    .unwrap()
    .unwrap()
}

#[tokio::test(threaded_scheduler)]
async fn test_metadata_from_wasm_api() {
    // test workspace boilerplate
    observability::test_run().ok();
    let env = holochain_state::test_utils::test_cell_env();
    let dbs = env.dbs().await;
    let env_ref = env.guard().await;
    let (base_entry_hash, target_entry_hash) = {
        clear_dbs(&env_ref, &dbs);
        let td = TestData::new().await;
        let base_entry_hash = td.original_entry_hash.clone();
        let target_entry_hash = td.new_entry_hash.clone();
        let (pre_state, expect, _) = register_add_link(td);
        Db::set(pre_state, &env_ref, &dbs).await;
        call_workflow(&env_ref, &dbs, env.clone()).await;
        Db::check(
            expect,
            &env_ref,
            &dbs,
            format!("{}: {}", "metadata from wasm", here!("")),
        )
        .await;
        (base_entry_hash, target_entry_hash)
    };
    let reader = holochain_state::env::ReadManager::reader(&env_ref).unwrap();
    let mut workspace = <crate::core::workflow::call_zome_workflow::InvokeZomeWorkspace as crate::core::state::workspace::Workspace>::new(&reader, &dbs).unwrap();

    let (_g, raw_workspace) =
        crate::core::workflow::unsafe_invoke_zome_workspace::UnsafeInvokeZomeWorkspace::from_mut(
            &mut workspace,
        );
    let mut host_context = HostContextFixturator::new(fixt::Unpredictable)
        .next()
        .unwrap();
    host_context.change_workspace(raw_workspace);
    let r = sync_call(Arc::new(host_context), base_entry_hash);
    assert_eq!(r[0].target, target_entry_hash);
}

#[tokio::test(threaded_scheduler)]
#[ignore]
async fn test_integrate_single_register_replaced_by_for_header() {
    // For RegisterReplacedBy with intended_for Header
    // metadata has EntryUpdate on HeaderHash but not EntryHash
    todo!()
}

#[tokio::test(threaded_scheduler)]
#[ignore]
async fn test_integrate_single_register_replaced_by_for_entry() {
    // For RegisterReplacedBy with intended_for Entry
    // metadata has EntryUpdate on EntryHash but not HeaderHash
    todo!()
}

#[tokio::test(threaded_scheduler)]
#[ignore]
async fn test_integrate_single_register_delete_on_headerd_by() {
    // For RegisterDeletedBy
    // metadata has ElementDelete on HeaderHash
    todo!()
}

#[tokio::test(threaded_scheduler)]
#[ignore]
async fn test_integrate_single_register_add_link() {
    // For RegisterAddLink
    // metadata has link on EntryHash
    todo!()
}

#[tokio::test(threaded_scheduler)]
#[ignore]
async fn test_integrate_single_register_remove_link() {
    // For RegisterAddLink
    // metadata has link on EntryHash
    todo!()
}

// TODO: Document this test
// TODO: Use the wasm calls directly instead of setting the databases to
// a state
// Integration
#[tokio::test(threaded_scheduler)]
async fn commit_entry_add_link() {
    observability::test_run().ok();
    let test_env = test_conductor_env();
    let _tmpdir = test_env.tmpdir.clone();
    let TestEnvironment {
        env: wasm_env,
        tmpdir: _tmpdir,
    } = test_wasm_env();
    let conductor = ConductorBuilder::new()
        .test(test_env, wasm_env)
        .await
        .unwrap();
    let shutdown = conductor.take_shutdown_handle().await.unwrap();
    let interface = RealAdminInterfaceApi::new(conductor.clone());
    let app_interface = RealAppInterfaceApi::new(conductor.clone());

    // Create dna
    let uuid = Uuid::new_v4();
    let dna = fake_dna_zomes(
        &uuid.to_string(),
        vec![(TestWasm::Foo.into(), TestWasm::Foo.into())],
    );

    // Install Dna
    let (fake_dna_path, _tmpdir) = write_fake_dna_file(dna.clone()).await.unwrap();
    let dna_payload = InstallAppDnaPayload::path_only(fake_dna_path, "".to_string());
    let agent_key = fake_agent_pubkey_1();
    let payload = InstallAppPayload {
        dnas: vec![dna_payload],
        app_id: "test".to_string(),
        agent_key: agent_key.clone(),
    };
    let request = AdminRequest::InstallApp(Box::new(payload));
    let r = interface.handle_admin_request(request).await;
    debug!(?r);
    let installed_app = unwrap_to!(r => AdminResponse::AppInstalled).clone();

    let cell_id = installed_app.cell_data[0].as_id().clone();
    // Activate app
    let request = AdminRequest::ActivateApp {
        app_id: installed_app.app_id,
    };
    let r = interface.handle_admin_request(request).await;
    assert_matches!(r, AdminResponse::AppActivated);

    let mut entry_fixt = SerializedBytesFixturator::new(Predictable).map(|b| Entry::App(b));

    let base_entry = entry_fixt.next().unwrap();
    let base_entry_hash = EntryHashed::with_data(base_entry.clone())
        .await
        .unwrap()
        .into_hash();
    let target_entry = entry_fixt.next().unwrap();
    let target_entry_hash = EntryHashed::with_data(target_entry.clone())
        .await
        .unwrap()
        .into_hash();
    // Put commit entry into source chain
    {
        let cell_env = conductor.get_cell_env(&cell_id).await.unwrap();
        let dbs = cell_env.dbs().await;
        let env_ref = cell_env.guard().await;

        let reader = env_ref.reader().unwrap();
        let mut sc = SourceChain::new(&reader, &dbs).unwrap();

        let header_builder = builder::EntryCreate {
            entry_type: EntryType::App(fixt!(AppEntryType)),
            entry_hash: base_entry_hash.clone(),
        };
        sc.put(header_builder, Some(base_entry.clone()))
            .await
            .unwrap();

        let header_builder = builder::EntryCreate {
            entry_type: EntryType::App(fixt!(AppEntryType)),
            entry_hash: target_entry_hash.clone(),
        };
        sc.put(header_builder, Some(target_entry.clone()))
            .await
            .unwrap();

        let header_builder = builder::LinkAdd {
            base_address: base_entry_hash.clone(),
            target_address: target_entry_hash.clone(),
            zome_id: 0.into(),
            tag: BytesFixturator::new(Unpredictable).next().unwrap().into(),
        };
        sc.put(header_builder, None).await.unwrap();
        env_ref
            .with_commit::<SourceChainError, _, _>(|writer| {
                sc.flush_to_txn(writer)?;
                Ok(())
            })
            .unwrap();
    }

    // Call zome to trigger a the produce workflow
    let request = Box::new(
        ZomeCallInvocationFixturator::new(NamedInvocation(
            cell_id.clone(),
            TestWasm::Foo,
            "foo".into(),
            HostInput::new(fixt!(SerializedBytes)),
        ))
        .next()
        .unwrap(),
    );
    let request = AppRequest::ZomeCallInvocation(request);
    let r = app_interface.handle_app_request(request).await;
    debug!(?r);

    tokio::time::delay_for(std::time::Duration::from_secs(4)).await;

    // Check the ops
    {
        let cell_env = conductor.get_cell_env(&cell_id).await.unwrap();
        let dbs = cell_env.dbs().await;
        let env_ref = cell_env.guard().await;

        let reader = env_ref.reader().unwrap();
        let db = dbs.get_db(&*INTEGRATED_DHT_OPS).unwrap();
        let ops_db = IntegratedDhtOpsStore::new(&reader, db).unwrap();
        let ops = ops_db.iter().unwrap().collect::<Vec<_>>().unwrap();
        debug!(?ops);
        assert!(!ops.is_empty());

        let meta = MetadataBuf::primary(&reader, &dbs).unwrap();
        let key = LinkMetaKey::Base(&base_entry_hash);
        let links = meta.get_links(&key).unwrap();
        let link = links[0].clone();
        assert_eq!(link.target, target_entry_hash);

        let (cas, _metadata, cache, metadata_cache) = test_dbs_and_mocks(&reader, &dbs);
        let cascade = Cascade::new(&cas, &meta, &cache, &metadata_cache);

        let links = cascade.dht_get_links(&key).await.unwrap();
        let link = links[0].clone();
        assert_eq!(link.target, target_entry_hash);

        let e = cascade.dht_get(&target_entry_hash).await.unwrap().unwrap();
        assert_eq!(e.into_content(), target_entry);

        let e = cascade.dht_get(&base_entry_hash).await.unwrap().unwrap();
        assert_eq!(e.into_content(), base_entry);
    }
    conductor.shutdown().await;
    shutdown.await.unwrap();
}