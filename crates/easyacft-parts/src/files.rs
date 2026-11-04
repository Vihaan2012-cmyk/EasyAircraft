//! Loading an aircraft from its files.

use crate::{Aircraft, Report};
use std::path::Path;

impl Aircraft {
    pub fn load(path: &Path) -> Result<(Self, Report), Report> {
        let (model, mut diags) = easyacft_lang::load(path);
        Self::from_model(model, &mut diags)
    }
}
