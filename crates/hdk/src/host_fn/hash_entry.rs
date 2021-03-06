use crate::prelude::*;

/// Hash anything that that implements TryInto<SerializedBytes> into an entry hash.
///
/// Hashes are typed in holochain, e.g. HeaderHash and EntryHash are different and yield different
/// bytes for a given value. This ensures correctness and allows type based dispatch in various
/// areas of the codebase.
///
/// Usually you want to hash a value that you want to reference on the DHT with `get` etc. because
/// it represents some domain-specific data sourced externally or generated within the wasm.
/// HeaderHash hashes are _always_ generated by the process of committing something to a local
/// chain. Every host function that commits an entry returns the new HeaderHash. The HeaderHash can
/// also be used with `get` etc. to retreive a _specific_ element from the DHT rather than the
/// oldest live element.
/// However there is no way to _generate_ a header hash directly from a header from inside wasm.
/// Element values (entry+header pairs returned by `get` etc.) contain prehashed header structs
/// called HeaderHashed, which is alongside the "raw" Header value. Generally the pre-hashing is
/// more efficient than hashing headers ad-hoc as hashing always needs to be done at the database
/// layer, so we want to re-use that as much as possible.
/// The header hash can be extracted from the Element as `element.header_hashed().as_hash()`.
/// @todo is there any use-case that can't be satisfied by the `header_hashed` approach?
///
/// Anything that is annotated with #[hdk_entry( .. )] or entry_def!( .. ) implements this so is
/// compatible automatically.
///
/// hash_entry is "dumb" in that it doesn't check the entry is defined, committed, on the DHT or
/// any other validation, it simply generates the hash for the serialized representation of
/// something in the same way that the DHT would.
///
/// It is strongly recommended that you use the `hash_entry` function to calculate hashes to avoid
/// inconsistencies between hashes in the wasm guest and the host.
/// For example, a lot of the crypto crates in rust compile to wasm so in theory could generate the
/// hash in the guest, but there is the potential that the serialization logic could be slightly
/// different, etc.
///
/// ```ignore
/// let foo_hash = hash_entry(foo)?;
/// ```
pub fn hash_entry<'a, I: 'a>(input: &'a I) -> HdkResult<EntryHash>
where
    SerializedBytes: TryFrom<&'a I, Error = SerializedBytesError>,
{
    let sb = SerializedBytes::try_from(input)?;
    Ok(host_call::<HashEntryInput, HashEntryOutput>(
        __hash_entry,
        &HashEntryInput::new(Entry::App(sb.try_into()?)),
    )?
    .into_inner())
}
