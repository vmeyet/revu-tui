//! The palette popup: the prompt with its mode, then MRs, answers or sessions to pick, or the commands that
//! match what is typed.
use super::app::App;
use super::palette::{Mode, Palette, Target, verbs_for};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Line;
use vtui::palette::view::{self, Pick, Popup};

pub(crate) fn draw(f: &mut Frame<'_>, app: &App, palette: &Palette, area: Rect) {
    let theme = &app.theme;
    let (sign, what) = palette.mode.prompt();
    let title = format!("search · {what}");
    let prompt = view::prompt(theme, palette, sign, |input| app.completions_for(input));
    let popup = Popup { title: &title, prompt, footer: footer(palette.mode) };
    view::draw(f, theme, area, popup, |room| match palette.mode {
        Mode::Commands => view::verb_lines(theme, palette, &verbs_for(&palette.input), room),
        Mode::Mrs | Mode::Answers | Mode::Sessions => list_lines(app, palette, room),
    });
}

fn list_lines(app: &App, palette: &Palette, room: usize) -> Vec<Line<'static>> {
    let theme = &app.theme;
    let candidates = app.palette_candidates(palette);
    let picks: Vec<Pick<'_>> = candidates
        .iter()
        .map(|candidate| {
            let (icon, colour) = match candidate.target {
                Target::Mr(_) => ("◆", theme.accent),
                Target::Answer(_) => ("✦", theme.accent),
                Target::Session(Some(_)) => ("●", theme.accent),
                Target::Session(None) => ("+", theme.muted),
            };
            Pick { icon, colour, label: &candidate.label, detail: &candidate.detail }
        })
        .collect();
    view::pick_lines(theme, &picks, palette.selected, room)
}

fn footer(mode: Mode) -> &'static str {
    match mode {
        Mode::Mrs => "  ↑↓ pick · enter opens · > commands · esc",
        Mode::Answers => "  ↑↓ pick · enter shows it · esc",
        Mode::Sessions => "  ↑↓ pick · enter opens it · esc",
        Mode::Commands => "  ↑↓ pick · tab completes · ^p history · esc",
    }
}
