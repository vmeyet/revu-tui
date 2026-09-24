//! `Y` and `:share`: pick where to post, add an optional note, read the message, then `y` sends it.
//! Nothing leaves before that `y`; `esc` at any step drops it all.
use super::{Action, App, Focus};
use crate::share::{Fields, Target, render};
use crate::tui::field::Field;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// A post being prepared for one MR.
#[derive(Clone, Debug, PartialEq)]
pub struct Sharing {
    pub fields: Fields,
    pub stage: Stage,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Stage {
    /// Several targets: the reader picks one.
    Pick { targets: Vec<Target>, selected: usize },
    /// The optional note, in a compose box.
    Note { target: Target, note: Field },
    /// The final message, waiting for `y`.
    Preview { target: Target, note: String, message: String },
}

impl Sharing {
    /// The modal's title: what is shared, and where once known.
    pub fn title(&self) -> String {
        match &self.stage {
            Stage::Pick { .. } => format!("share {}", self.fields.reference),
            Stage::Note { target, .. } | Stage::Preview { target, .. } => match &target.name {
                Some(name) => format!("share {} to {name}", self.fields.reference),
                None => format!("share {}", self.fields.reference),
            },
        }
    }
}

impl App {
    /// `Y` or `:share [target]` on the MR under the cursor: the queue's row, or the open MR.
    pub(super) fn start_share(&mut self, wanted: Option<&str>) {
        if self.share_targets.is_empty() {
            self.toast("nothing to share to: add `[share] command` to the config (docs/guides/share.md)");
            return;
        }
        let Some(fields) = self.share_fields() else {
            self.toast("select an MR first");
            return;
        };
        let targets = self.share_targets.clone();
        let stage = if let Some(name) = wanted {
            let Some(target) = targets.iter().find(|t| t.name.as_deref() == Some(name)) else {
                self.toast(format!("no share target named {name}: {}", target_names(&targets)));
                return;
            };
            note_stage(target.clone())
        } else if targets.len() == 1 {
            note_stage(targets[0].clone())
        } else {
            Stage::Pick { targets, selected: 0 }
        };
        self.sharing = Some(Sharing { fields, stage });
    }

    fn share_fields(&self) -> Option<Fields> {
        match (self.focus, &self.open) {
            (Focus::Queue, _) | (_, None) => self.selected_mr().map(|mr| Fields::from_queue(mr, self.hosts.kind_of(&mr.key()))),
            (_, Some(open)) => Some(Fields::from_mr(&open.review.mr, self.hosts.kind_of(&open.key))),
        }
    }

    pub(super) fn handle_share_key(&mut self, key: KeyEvent) -> Vec<Action> {
        let Some(sharing) = self.sharing.take() else { return vec![] };
        if key.code == KeyCode::Esc {
            self.toast("not shared");
            return vec![];
        }
        let Sharing { fields, stage } = sharing;
        let stage = match stage {
            Stage::Pick { targets, selected } => pick_key(targets, selected, key),
            Stage::Note { target, note } => note_key(&fields, target, note, key),
            Stage::Preview { target, note, message } => match key.code {
                KeyCode::Char('y') => {
                    let done = target.done(&fields.reference);
                    return vec![Action::Share { target: Box::new(target), message, done }];
                }
                KeyCode::Char('e') => Stage::Note { target, note: Field::new(note) },
                _ => Stage::Preview { target, note, message },
            },
        };
        self.sharing = Some(Sharing { fields, stage });
        vec![]
    }
}

fn note_stage(target: Target) -> Stage {
    Stage::Note { target, note: Field::default() }
}

fn pick_key(targets: Vec<Target>, selected: usize, key: KeyEvent) -> Stage {
    let last = targets.len().saturating_sub(1);
    let digit = match key.code {
        KeyCode::Char(c @ '1'..='9') => c.to_digit(10).map(|d| d as usize - 1).filter(|&i| i <= last),
        _ => None,
    };
    match (key.code, digit) {
        (_, Some(index)) => note_stage(targets[index].clone()),
        (KeyCode::Enter, _) => note_stage(targets[selected].clone()),
        (KeyCode::Char('j') | KeyCode::Down, _) => Stage::Pick { selected: (selected + 1).min(last), targets },
        (KeyCode::Char('k') | KeyCode::Up, _) => Stage::Pick { selected: selected.saturating_sub(1), targets },
        _ => Stage::Pick { targets, selected },
    }
}

/// The note box reads like the compose box: `enter` goes on to the preview, even with no note.
fn note_key(fields: &Fields, target: Target, mut note: Field, key: KeyEvent) -> Stage {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let newline = key.modifiers.intersects(KeyModifiers::ALT | KeyModifiers::SHIFT);
    match key.code {
        KeyCode::Enter if newline => note.insert('\n'),
        KeyCode::Enter => {
            let note = note.take().trim().to_owned();
            let message = render(&target.template, fields, &note);
            return Stage::Preview { target, note, message };
        }
        KeyCode::Char('j') if ctrl => note.insert('\n'),
        KeyCode::Left => note.left(),
        KeyCode::Right => note.right(),
        KeyCode::Home => note.start(),
        KeyCode::End => note.end(),
        KeyCode::Char('a') if ctrl => note.start(),
        KeyCode::Char('e') if ctrl => note.end(),
        KeyCode::Char('w') if ctrl => note.delete_word(),
        KeyCode::Backspace => note.backspace(),
        KeyCode::Delete => note.delete(),
        KeyCode::Char(c) if !ctrl => note.insert(c),
        _ => {}
    }
    Stage::Note { target, note }
}

fn target_names(targets: &[Target]) -> String {
    let names: Vec<&str> = targets.iter().filter_map(|t| t.name.as_deref()).collect();
    if names.is_empty() { "only the bare [share] target exists".to_owned() } else { format!("the targets are {}", names.join(", ")) }
}
