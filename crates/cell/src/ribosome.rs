use crate::{
    nucleus::{ZomeInvocation, ZomeInvocationResult},
    wasm_engine::WasmEngine,
};
use mockall::automock;
use sx_types::{dna::Dna, entry::Entry, error::SkunkResult, shims::*};

#[automock]
pub trait RibosomeT {
    fn run_validation(self, entry: Entry) -> ValidationResult;

    /// Runs the specified zome fn. Returns the cursor used by HDK,
    /// so that it can be passed on to source chain manager for transactional writes
    ///
    /// Note: it would be nice to pass the bundle by value and then return it at the end,
    /// but automock doesn't support lifetimes that appear in return values
    fn call_zome_function<'env>(
        self,
        bundle: &mut SourceChainCommitBundle<'env>,
        invocation: ZomeInvocation,
        // source_chain: SourceChain,
    ) -> SkunkResult<ZomeInvocationResult>;
}

/// TODO determine what cursor looks like for ribosomes
/// Total hack just to have something to look at
/// The only Ribosome is a Wasm ribosome.
pub struct Ribosome {
    engine: WasmEngine,
}

impl Ribosome {
    pub fn new(dna: Dna) -> Self {
        Self { engine: WasmEngine }
    }
}

impl RibosomeT for Ribosome {
    fn run_validation(self, entry: Entry) -> ValidationResult {
        unimplemented!()
    }

    /// Runs the specified zome fn. Returns the cursor used by HDK,
    /// so that it can be passed on to source chain manager for transactional writes
    fn call_zome_function<'env>(
        self,
        bundle: &mut SourceChainCommitBundle<'env>,
        invocation: ZomeInvocation,
        // source_chain: SourceChain,
    ) -> SkunkResult<ZomeInvocationResult> {
        unimplemented!()
    }
}