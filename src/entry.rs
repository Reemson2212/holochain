use crate::Address;
use crate::Dna;
use crate::LinkData;
use crate::PublicKey;
use crate::Timestamp;

pub enum DHTElement {
    Header(Header),
    HeaderWithEntry(HeaderWithEntry),
}

pub enum HeaderWithEntry {
    Create(NormalHeader, BigEEntry),
    Update(UpdateHeader, BigEEntry),
    Delete(NormalHeader, DeleteEntry),
    AddLink(NormalHeader, AddLinkEntry),
    RemoveLink(NormalHeader, RemoveLinkEntry),
}

impl HeaderWithEntry {
    pub fn try_from_parts(header: Header, entry: Entry) -> Result<Self, (Header, Entry)> {
        Ok(match (header, entry) {
            (Header::Create(header), Entry::BigE(entry)) => Self::Create(header, entry),
            (Header::Update(header), Entry::BigE(entry)) => Self::Update(header, entry),
            (Header::Delete(header), Entry::Delete(entry)) => Self::Delete(header, entry),
            (Header::AddLink(header), Entry::AddLink(entry)) => Self::AddLink(header, entry),
            (Header::RemoveLink(header), Entry::RemoveLink(entry)) => {
                Self::RemoveLink(header, entry)
            }

            (header, entry) => return Err((header, entry)),
        })
    }

    pub fn into_header(self) -> Header {
        use HeaderWithEntry::*;

        match self {
            Create(header, _) => Header::Create(header),
            Update(header, _) => Header::Update(header),
            Delete(header, _) => Header::Delete(header),
            AddLink(header, _) => Header::AddLink(header),
            RemoveLink(header, _) => Header::RemoveLink(header),
        }
    }
}

pub enum ChainHeaderWithEntry {
    Normal(HeaderWithEntry),
    Dna(MinHeader, Dna),
}

impl ChainHeaderWithEntry {
    pub fn try_from_parts(header: Header, entry: ChainEntry) -> Result<Self, (Header, ChainEntry)> {
        Ok(match entry {
            ChainEntry::Dna(entry) => match header {
                Header::Dna(header) => Self::Dna(header, entry),
                header => return Err((header, ChainEntry::Dna(entry))),
            },
            ChainEntry::Normal(entry) => Self::Normal(
                HeaderWithEntry::try_from_parts(header, entry)
                    .map_err(|(header, entry)| (header, ChainEntry::Normal(entry)))?,
            ),
        })
    }
}

pub enum Header {
    Dna(MinHeader),
    Create(NormalHeader),
    Update(UpdateHeader),
    Delete(NormalHeader),
    AddLink(NormalHeader),
    RemoveLink(NormalHeader),
}

impl Header {
    pub fn author(&self) -> &PublicKey {
        use Header::*;

        match &self {
            Dna(min_header) => &min_header.author,
            Create(normal_header)
            | Delete(normal_header)
            | AddLink(normal_header)
            | RemoveLink(normal_header) => &normal_header.author,

            Update(update_header) => &update_header.author,
        }
    }
}

pub enum Entry {
    BigE(BigEEntry),
    Header(Header),
    Delete(DeleteEntry),
    AddLink(AddLinkEntry),
    RemoveLink(RemoveLinkEntry),
}

pub enum ChainEntry {
    Dna(Dna),
    Normal(Entry),
}

pub struct UpdateHeader {
    pub replaces: Address,
    pub prev_header: Address,
    pub entry: Address,
    pub author: PublicKey,
    pub timestamp: Timestamp,
}

pub struct NormalHeader {
    pub prev_header: Address,
    pub entry: Address,
    pub author: PublicKey,
    pub timestamp: Timestamp,
}

pub struct MinHeader {
    pub entry: Address,
    pub author: PublicKey,
    pub timestamp: Timestamp,
}

// TODO: where do private entries fit in?
#[derive(Clone)]
pub enum BigEEntry {
    App(AppEntry),
    AgentKey(PublicKey),
}
// TODO: What goes here?
#[derive(Clone)]
pub struct AppEntry;

pub struct DeleteEntry {
    pub deletes: Address,
}

pub struct AddLinkEntry {
    pub base: Address,
    pub target: Address,
    pub data: LinkData,
}

pub struct RemoveLinkEntry {
    pub base: Address,
    /// Address of the AddLinkEntry to be removed
    pub removes: Address,
}