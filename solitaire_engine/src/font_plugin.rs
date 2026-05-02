//! Embeds FiraMono-Medium into the binary and exposes it via [`FontResource`].
//!
//! Bundling rather than runtime-loading guarantees the canonical UI face is
//! always available regardless of install or platform. The bytes are
//! validated at startup; a parse failure aborts the program with a clear
//! error because it means the binary is corrupt.

use bevy::prelude::*;

/// FiraMono-Medium bytes embedded at compile time. Single source of truth for
/// the project's UI face — `solitaire_engine::assets::svg_loader` embeds the
/// same path independently for SVG rasterisation so the two layers can't
/// drift.
const BUNDLED_FONT_BYTES: &[u8] = include_bytes!("../../assets/fonts/main.ttf");

/// Holds the project-wide [`Handle<Font>`] registered at startup.
#[derive(Resource)]
pub struct FontResource(pub Handle<Font>);

/// Registers the bundled FiraMono with [`Assets<Font>`] at startup.
pub struct FontPlugin;

impl Plugin for FontPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, load_font);
    }
}

fn load_font(fonts: Option<ResMut<Assets<Font>>>, mut commands: Commands) {
    // Headless test fixtures use MinimalPlugins (no AssetPlugin → no
    // Assets<Font>). FontPlugin in that context is a no-op — consumers
    // already query `Option<Res<FontResource>>` and degrade cleanly.
    let Some(mut fonts) = fonts else { return };
    let font = Font::try_from_bytes(BUNDLED_FONT_BYTES.to_vec())
        .expect("bundled FiraMono failed to parse — binary is corrupt");
    let handle = fonts.add(font);
    commands.insert_resource(FontResource(handle));
}
