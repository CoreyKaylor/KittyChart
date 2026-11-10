//! Terminal color themes: built-ins, plus Ghostty-format theme files.

use std::path::Path;

#[derive(Debug, Clone)]
pub struct Theme {
    pub name: String,
    pub background: [u8; 3],
    pub foreground: [u8; 3],
    pub cursor: [u8; 3],
    /// ANSI 0–15.
    pub palette: [[u8; 3]; 16],
}

fn hex(s: &str) -> Option<[u8; 3]> {
    let s = s.trim().trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let p = |i: usize| u8::from_str_radix(&s[i..i + 2], 16).ok();
    Some([p(0)?, p(2)?, p(4)?])
}

fn theme(name: &str, bg: &str, fg: &str, palette: [&str; 16]) -> Theme {
    let fgc = hex(fg).expect("valid builtin");
    Theme {
        name: name.into(),
        background: hex(bg).expect("valid builtin"),
        foreground: fgc,
        cursor: fgc,
        palette: palette.map(|c| hex(c).expect("valid builtin")),
    }
}

pub const BUILTIN: &[&str] = &["tokyonight", "ghostty", "kitty", "github-light", "solarized-light"];

pub fn builtin(name: &str) -> Option<Theme> {
    Some(match name {
        "tokyonight" => theme(
            "tokyonight",
            "#1a1b26",
            "#a9b1d6",
            [
                "#1a1b26", "#f7768e", "#9ece6a", "#e0af68", "#7aa2f7", "#ad8ee6", "#449dab", "#a9b1d6",
                "#414868", "#ff7a93", "#b9f27c", "#ff9e64", "#7da6ff", "#bb9af7", "#0db9d7", "#c0caf5",
            ],
        ),
        "ghostty" => theme(
            "ghostty",
            "#282c34",
            "#ffffff",
            [
                "#1d1f21", "#cc6666", "#b5bd68", "#f0c674", "#81a2be", "#b294bb", "#8abeb7", "#c5c8c6",
                "#666666", "#d54e53", "#b9ca4a", "#e7c547", "#7aa6da", "#c397d8", "#70c0b1", "#eaeaea",
            ],
        ),
        "kitty" => theme(
            "kitty",
            "#000000",
            "#dddddd",
            [
                "#000000", "#cc0403", "#19cb00", "#cecb00", "#0d73cc", "#cb1ed1", "#0dcdcd", "#dddddd",
                "#767676", "#f2201f", "#23fd00", "#fffd00", "#1a8fff", "#fd28ff", "#14ffff", "#ffffff",
            ],
        ),
        "github-light" => theme(
            "github-light",
            "#ffffff",
            "#1f2328",
            [
                "#24292f", "#cf222e", "#116329", "#4d2d00", "#0969da", "#8250df", "#1b7c83", "#6e7781",
                "#57606a", "#a40e26", "#1a7f37", "#633c01", "#218bff", "#a475f9", "#3192aa", "#8c959f",
            ],
        ),
        "solarized-light" => theme(
            "solarized-light",
            "#fdf6e3",
            "#657b83",
            [
                "#073642", "#dc322f", "#859900", "#b58900", "#268bd2", "#d33682", "#2aa198", "#eee8d5",
                "#002b36", "#cb4b16", "#586e75", "#657b83", "#839496", "#6c71c4", "#93a1a1", "#fdf6e3",
            ],
        ),
        _ => return None,
    })
}

/// Parse a Ghostty theme/config file (`background = #…`, `palette = N=#…`).
pub fn from_ghostty_file(path: &Path) -> Result<Theme, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut t = builtin("ghostty").expect("builtin");
    t.name = path.file_stem().map_or("custom".into(), |s| s.to_string_lossy().into_owned());
    let mut cursor = None;
    for line in text.lines() {
        let Some((k, v)) = line.split_once('=') else { continue };
        let (k, v) = (k.trim(), v.trim());
        match k {
            "background" => t.background = hex(v).unwrap_or(t.background),
            "foreground" => t.foreground = hex(v).unwrap_or(t.foreground),
            "cursor-color" => cursor = hex(v),
            "palette" => {
                if let Some((i, c)) = v.split_once('=')
                    && let (Ok(i), Some(c)) = (i.trim().parse::<usize>(), hex(c))
                    && i < 16
                {
                    t.palette[i] = c;
                }
            }
            _ => {}
        }
    }
    t.cursor = cursor.unwrap_or(t.foreground);
    Ok(t)
}

/// A built-in name, or a path to a Ghostty theme file.
pub fn resolve(spec: &str) -> Result<Theme, String> {
    builtin(spec).map_or_else(|| from_ghostty_file(Path::new(spec)), Ok)
}
