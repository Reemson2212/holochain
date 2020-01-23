use crate::entry::{
    AddLinkEntry, BigEEntry, DeleteEntry, Header, HeaderWithEntry, NormalHeader,
    RemoveLinkEntry, UpdateHeader,
};
use crate::{hash, Address, Signed};

pub enum Notification {
    #[cfg(feature = "strict")]
    StoreHeader(Signed<HeaderWithEntry>),
    #[cfg(not(feature = "strict"))]
    StoreHeader(Signed<Header>),
    StoreEntry(Signed<NormalHeader>, BigEEntry),
    RegisterAgentActivity(Signed<Header>),
    RegisterUpdatedTo(Signed<UpdateHeader>, BigEEntry),
    RegisterDeletedBy(Signed<NormalHeader>, DeleteEntry),
    RegisterAddLink(Signed<NormalHeader>, AddLinkEntry),
    RegisterRemoveLink(Signed<NormalHeader>, RemoveLinkEntry),
}

pub fn hash_hood(notification: Notification) -> Address {
    use Notification::*;

    match notification {
        #[cfg(feature = "strict")]
        StoreHeader(header_with_entry) => hash(header_with_entry.into_inner().into_header()),
        #[cfg(not(feature = "strict"))]
        StoreHeader(header) => hash(header),
        StoreEntry(header, _entry) => header.into_inner().entry,
        RegisterAgentActivity(header) => hash(header.author()),
        RegisterUpdatedTo(header, _entry) => header.into_inner().replaces,
        RegisterDeletedBy(_header, entry) => entry.deletes,
        RegisterAddLink(_header, entry) => entry.base,
        RegisterRemoveLink(_header, entry) => entry.base,
    }
}

// // FIXME: avoid allocation or somehow give caller the choice
// fn produce_notifications_from_commit(commit: Signed<ChainHeaderWithEntry>) -> Vec<Notification> {
//     let Signed {sig, data: header_with_entry} = commit;
//     let fourth_notification = match header_with_entry {
//         Dna(header, _dna) => {
//             // dna is never published
//             let header = Notification::Sto
//         }
//         Create => {
//             return vec![store_header, store_entry, register_agent_activity];
//         }
//         Update => {
//             let register_updated_to = Notification {
//                 // FIXME: avoid cloning
//                 header: header.clone(),
//                 data: NotificationData::RegisterUpdatedTo {
//                     entry: entry.clone(),
//                 },
//             };
//             register_updated_to
//         }
//         Delete => {
//             let register_deleted_by = Notification {
//                 header: header.clone(),
//                 data: NotificationData::RegisterDeletedBy {
//                     entry: entry.clone(),
//                 },
//             };
//             register_deleted_by
//         }
//         AddLink => {
//             let register_add_link = Notification {
//                 header: header.clone(),
//                 data: NotificationData::RegisterAddLink {
//                     entry: entry.clone(),
//                 },
//             };
//             register_add_link
//         }
//         RemoveLink => {
//             let register_remove_link = Notification {
//                 header: header.clone(),
//                 data: NotificationData::RegisterRemoveLink {
//                     entry: entry.clone(),
//                 },
//             };
//             register_remove_link
//         }
//     };
//
//     #[cfg(feature = "strict")]
//     let store_header = Notification::StoreHeader {
//
//     };
//     let store_entry = Notification {
//         // FIXME: avoid cloning
//         header: header.clone(),
//         data: NotificationData::StoreEntry {
//             // FIXME: avoid cloning
//             entry: entry.clone(),
//         },
//     };
//     let register_agent_activity = Notification {
//         // FIXME: avoid cloning
//         header: header.clone(),
//         data: NotificationData::RegisterAgentActivity,
//     };
//
//     vec![
//         store_header,
//         store_entry,
//         register_agent_activity,
//         fourth_notification,
//     ]
// }
//
// #[derive(Hash)]
// enum HashReadyForm<'a> {
//     // optimization: don't hash signature. it is redundant with header and therefore wastes hash time to include
//     StoredHeader {
//         header: &'a HeaderWithoutSig,
//     },
//     StoredEntry {
//         header: &'a HeaderWithoutSig,
//     },
//     RegisteredAgentActivity {
//         header: &'a HeaderWithoutSig,
//     },
//     RegisteredUpdatedTo {
//         entry: &'a Entry,
//         replaces: &'a Address,
//     },
//     RegisteredDeletedBy {
//         entry: &'a DeleteEntry,
//     },
//     RegisteredAddLink {
//         header: &'a HeaderWithoutSig,
//     },
//     // ^ future work: encode idempotency in LinkAdd entries themselves
//     RegisteredRemoveLink {
//         header: &'a HeaderWithoutSig,
//     },
//     // ^ if LinkAdds were idempotent then this could just be entry.
// }
//
// fn unique_hash(notification: &Notification) -> HashReadyForm<'_> {
//     use NotificationData::*;
//
//     let Notification { header, data } = notification;
//     match data {
//         StoreHeader { .. } => HashReadyForm::StoredHeader { header },
//         StoreEntry { .. } => HashReadyForm::StoredEntry { header },
//         RegisterAgentActivity => HashReadyForm::RegisteredAgentActivity { header },
//         RegisterUpdatedTo { entry } => HashReadyForm::RegisteredUpdatedTo {
//             entry,
//             replaces: header.replaces(),
//         },
//         RegisterDeletedBy { entry } => HashReadyForm::RegisteredDeletedBy { entry },
//         RegisterAddLink { .. } => HashReadyForm::RegisteredAddLink { header },
//         RegisterRemoveLink { .. } => HashReadyForm::RegisteredRemoveLink { header },
//     }
// }