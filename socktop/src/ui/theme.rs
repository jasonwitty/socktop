//! Shared UI theme constants.

use ratatui::style::Color;

// Memory gauge segments (see ui/mem.rs): used / reclaimable cache / free.
pub const MEM_USED: Color = Color::Magenta;
pub const MEM_CACHE: Color = Color::Rgb(102, 153, 204); // steel blue
pub const MEM_FREE: Color = Color::Rgb(70, 70, 85); // barely-there remainder

// GPU pane (see ui/gpu.rs): bars, labels, and the card tab strip.
pub const GPU_UTIL: Color = Color::Green;
pub const GPU_VRAM: Color = Color::LightMagenta;
pub const GPU_TEXT: Color = Color::Gray;
// Unselected card tabs sit back; the selected one gets a filled block so the
// current card is obvious without hunting for it.
pub const GPU_TAB: Color = Color::DarkGray;
pub const GPU_TAB_TEXT: Color = Color::Black;
pub const GPU_TAB_BG: Color = Color::Gray;

// Scrollbar colors (same look as before)
pub const SB_ARROW: Color = Color::Rgb(170, 170, 180);
pub const SB_TRACK: Color = Color::Rgb(170, 170, 180);
pub const SB_THUMB: Color = Color::Rgb(170, 170, 180);

// Modal palette
pub const MODAL_DIM_BG: Color = Color::Rgb(15, 15, 25);
pub const MODAL_BG: Color = Color::Rgb(26, 26, 46);
pub const MODAL_FG: Color = Color::Rgb(230, 230, 230);
pub const MODAL_TITLE_FG: Color = Color::Rgb(255, 102, 102); // soft red for title text
pub const MODAL_BORDER_FG: Color = Color::Rgb(204, 51, 51); // darker red border

pub const MODAL_ICON_PINK: Color = Color::Rgb(255, 182, 193); // light pink icons line
pub const MODAL_AGENT_FG: Color = Color::Rgb(220, 220, 255); // pale periwinkle
pub const MODAL_HINT_FG: Color = Color::Rgb(255, 215, 0); // gold for message icon
pub const MODAL_OFFLINE_LABEL_FG: Color = Color::Rgb(135, 206, 235); // sky blue label
pub const MODAL_RETRY_LABEL_FG: Color = Color::Rgb(255, 165, 0); // orange label
pub const MODAL_COUNTDOWN_LABEL_FG: Color = Color::Rgb(255, 192, 203); // pink label for countdown

// Buttons
pub const BTN_RETRY_BG_ACTIVE: Color = Color::Rgb(46, 204, 113); // modern green
pub const BTN_RETRY_FG_ACTIVE: Color = Color::Rgb(26, 26, 46);
pub const BTN_RETRY_FG_INACTIVE: Color = Color::Rgb(46, 204, 113);

pub const BTN_EXIT_BG_ACTIVE: Color = Color::Rgb(255, 255, 255); // modern red
pub const BTN_EXIT_FG_ACTIVE: Color = Color::Rgb(26, 26, 46);
pub const BTN_EXIT_FG_INACTIVE: Color = Color::Rgb(255, 255, 255);

// Process selection colors
pub const PROCESS_SELECTION_BG: Color = Color::Rgb(147, 112, 219); // Medium slate blue (purple)
pub const PROCESS_SELECTION_FG: Color = Color::Rgb(255, 255, 255); // White text for contrast
pub const PROCESS_TOOLTIP_BG: Color = Color::Rgb(147, 112, 219); // Same purple as selection
pub const PROCESS_TOOLTIP_FG: Color = Color::Rgb(255, 255, 255); // White text for contrast

// Process details modal colors (matches main UI aesthetic - no custom colors, terminal defaults)
pub const PROCESS_DETAILS_ACCENT: Color = Color::Rgb(147, 112, 219); // Purple accent for highlights

// Emoji / icon strings (centralized so they can be themed/swapped later)
pub const ICON_WARNING_TITLE: &str = " 🔌 CONNECTION ERROR ";
pub const ICON_CLUSTER: &str = "⚠️";
pub const ICON_MESSAGE: &str = "💭 ";
pub const ICON_OFFLINE_LABEL: &str = "⏱️  Offline for: ";
pub const ICON_RETRY_LABEL: &str = "🔄 Retry attempts: ";
pub const ICON_COUNTDOWN_LABEL: &str = "⏰ Next auto retry: ";
pub const BTN_RETRY_TEXT: &str = " 🔄 Retry ";
pub const BTN_EXIT_TEXT: &str = " ❌ Exit ";

// warning icon
pub const LARGE_ERROR_ICON: &[&str] = &[
    "           /\\           ",
    "          /  \\          ",
    "         / !! \\         ",
    "        / !!!! \\        ",
    "       /  !!    \\       ",
    "      /   !!!!   \\      ",
    "     /     !!     \\     ",
    "    /______________\\    ",
];

//about logo
pub const ASCII_ART: &str = r#"
⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⣀⣠⡄⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀
⠀⠀⠀⠀⠀⠀⠀⠀⣀⣤⣶⣾⠿⠿⠛⠃⠀⠀⠀⠀⠀⣀⣀⣠⡄⠀⠀⠀⠀⠀
⠀⠀⠀⠀⠀⠀⠀⠀⠘⠛⢉⣠⣴⣾⣿⠿⠆⢰⣾⡿⠿⠛⠛⠋⠁⠀⠀⠀⠀⠀
⠀⠀⠀⠀⠀⠀⠀⠀⠀⠘⣿⠟⠋⣁⣤⣤⣶⠀⣠⣤⣶⣾⣿⣿⡿⠀⠀⠀⠀⠀
⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⣶⣿⣿⣿⣿⣿⡆⠘⠛⢉⣁⣤⣤⣤⡀⠀⠀⠀⠀
⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⢸⣿⣿⣿⣿⣿⣿⡀⢾⣿⣿⣿⣿⣿⡇⠀⠀⠀⠀
⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⣿⣿⣿⣿⣿⣿⣧⠈⢿⣿⣿⣿⣿⣷⠀⠀⠀⠀
⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⢸⣿⣿⣿⣿⣿⣿⣧⠈⢿⣿⣿⣿⣿⡄⠀⠀⠀
⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⣼⣿⣿⣿⣿⣿⠿⠋⣁⠀⢿⣿⣿⣿⣷⡀⠀⠀
⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⣠⣴⣿⣿⣿⣿⡟⢁⣴⣿⣿⡇⢸⣿⣿⡿⠟⠃⠀⠀
⠀⠀⠀⠀⠀⠀⢀⣠⣴⣿⣿⣿⣿⣿⣿⡟⢀⣿⣿⣿⡟⢀⣾⠟⢁⣤⣶⣿⠀⠀
⠀⠀⠀⠀⣠⣶⣿⣿⣿⣿⣿⣿⣿⣿⣿⡇⠸⡿⠟⢋⣠⣾⠃⣰⣿⣿⣿⡟⠀⠀
⠀⠀⣴⣄⠙⣿⣿⣿⣿⣿⡿⠿⠛⠋⣉⣁⣤⣴⣶⣿⣿⣿⠀⣿⡿⠟⠋⠀⠀⠀
⠀⠀⣿⣿⡆⠹⠟⠋⣁⣤⡄⢰⣿⠿⠟⠛⠋⠉⠉⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀
⠀⠀⠈⠉⠁⠀⠀⠀⠙⠛⠃⠈⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀

███████╗ ██████╗  ██████╗████████╗ ██████╗ ██████╗ 
██╔════╝██╔═══██╗██╔════╝╚══██╔══╝██╔═══██╗██╔══██╗
███████╗██║   ██║██║        ██║   ██║   ██║██████╔╝
╚════██║██║   ██║██║        ██║   ██║   ██║██╔═══╝ 
███████║╚██████╔╝╚██████╗   ██║   ╚██████╔╝██║     
╚══════╝ ╚═════╝  ╚═════╝   ╚═╝    ╚═════╝ ╚═╝     
"#;
