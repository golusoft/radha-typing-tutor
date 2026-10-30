//! The layout registry: the production entry point.
//!
//! The only module (besides `lib.rs`) that names every layout.

use crate::cbi::CbiLayout;
use crate::error::LayoutError;
use crate::gail::GailLayout;
use crate::inscript::InscriptLayout;
use crate::krutidev::KrutiDevLayout;
use crate::qwerty::QwertyLayout;
use crate::{KeyboardLayout, LayoutId};

static QWERTY: QwertyLayout = QwertyLayout::new();
static GAIL: GailLayout = GailLayout::new();
static CBI: CbiLayout = CbiLayout::new();
static KRUTIDEV: KrutiDevLayout = KrutiDevLayout::new();
static INSCRIPT: InscriptLayout = InscriptLayout::new();

/// The layout for `id`, after validating its table.
///
/// # Errors
///
/// [`LayoutError::InvalidTable`] when the layout's table breaks a rule; the
/// app treats that layout as unavailable and keeps running.
pub fn load_layout(id: LayoutId) -> Result<&'static dyn KeyboardLayout, LayoutError> {
    let layout: &'static dyn KeyboardLayout = match id {
        LayoutId::Qwerty => &QWERTY,
        LayoutId::RemingtonGail => &GAIL,
        LayoutId::RemingtonCbi => &CBI,
        LayoutId::KrutiDev => &KRUTIDEV,
        LayoutId::Inscript => &INSCRIPT,
    };
    layout.table().validate()?;
    if let Some(dead) = layout.dead_keys() {
        dead.validate()?;
    }
    Ok(layout)
}
