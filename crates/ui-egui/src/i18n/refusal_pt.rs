//! Portuguese (Brazil): one sentence per refusal code, shown before the engine's English detail.

use openlaser_types::RefusalCode;

/// Every refusal code, so the test can check each one has a sentence.
#[cfg(test)]
pub(super) const ALL_CODES: &[RefusalCode] = &[RefusalCode::OutsideWorkArea];

pub(super) fn get(code: RefusalCode) -> Option<&'static str> {
    use RefusalCode::*;
    Some(match code {
        OutsideWorkArea => "Fora da área de trabalho.",
        _ => return None,
    })
}
