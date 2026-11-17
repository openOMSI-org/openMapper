//! Keeping one editor's map and the host's the same: what this editor changed goes to the
//! host one edit at a time; what the others changed comes in and is put in.
//!
//! Edits are made at once on this editor's map and sent after; one is on its way at a time,
//! and what is changed meanwhile goes with the next (with the versions the host gave the one
//! before). An edit the host turns down (somebody changed one of its parts first) is put
//! back to the host's state, and so is an unsent change of a part somebody else's edit
//! replaces. Undo forgets the steps on parts somebody else changed: undoing them would put
//! back what was there before that person's edit.

use std::collections::{BTreeSet, HashMap};

use openmapper_collab::{Msg, Part, State, Why};

use super::codec;
use crate::doc::Document;
use crate::history::{Changed, History};

/// What the editor tells the person after a message.
#[derive(Debug, Clone, PartialEq)]
pub enum Note {
    /// An edit of theirs was turned down, and why (`tiles` tiles put back as the host has
    /// them).
    TurnedDown { tiles: usize, why: Why },
    /// Somebody's edit replaced changes of theirs not sent yet.
    Replaced { by: u32, parts: usize },
    /// A guest changed the timetable, the traffic or the descriptions: put back.
    HostOnly,
    /// A part could not be read (an editor of another version?): left as it was.
    Unreadable(Part),
    Recovered(std::path::PathBuf),
    RecoveryFailed(String),
}

pub struct Sync {
    /// The version of each part as the host last said.
    known: HashMap<Part, u64>,
    /// Changed here, not sent yet.
    dirty: BTreeSet<Part>,
    /// The edit on its way: its number and parts.
    flight: Option<(u64, Vec<Part>)>,
    seq: u64,
    /// This editor hosts (the timetable, the traffic and the descriptions are its own).
    host: bool,
    /// The IDCodes this editor hands out now, and the next block once it came.
    ids: (i64, i64),
    spare: Option<(i64, i64)>,
    asked: bool,
}

impl Sync {
    pub fn new(states: &[State], ids: (i64, i64), host: bool, doc: &mut Document) -> Sync {
        doc.global.next_id_code = ids.0;
        Sync { known: states.iter().map(|s| (s.part, s.version)).collect(), dirty: BTreeSet::new(), flight: None, seq: 0, host, ids, spare: None, asked: false }
    }

    /// Whether everything of this editor's has reached the host.
    pub fn settled(&self) -> bool {
        self.dirty.is_empty() && self.flight.is_none()
    }

    /// What the history says was changed here (`History::take_changed`).
    pub fn edited(&mut self, ch: Changed, doc: &mut Document, history: &mut History) -> Option<Note> {
        self.mark(&ch);
        if !self.host && (ch.timetable || ch.traffic || ch.descriptions) {
            // (those are kept by the host alone: a guest's change would never reach it)
            history.undo(doc);
            let back = history.take_changed(doc);
            self.mark(&back);
            return Some(Note::HostOnly);
        }
        None
    }

    fn mark(&mut self, ch: &Changed) {
        self.dirty.extend(ch.tiles.iter().map(|k| Part::Tile(k.0, k.1)));
        if ch.global {
            self.dirty.insert(Part::Global);
        }
        if ch.groups {
            self.dirty.insert(Part::Groups);
        }
    }

    /// The next edit to send, when none is on its way.
    pub fn next_edit(&mut self, doc: &Document) -> Option<Msg> {
        if self.flight.is_some() || self.dirty.is_empty() {
            return None;
        }
        let parts: Vec<Part> = std::mem::take(&mut self.dirty).into_iter().collect();
        let states = parts.iter().map(|&part| State { part, version: 0, data: bytes_of(doc, part) }).collect();
        let base = parts.iter().map(|p| (*p, self.known.get(p).copied().unwrap_or(0))).collect();
        self.seq += 1;
        self.flight = Some((self.seq, parts));
        Some(Msg::Edit { seq: self.seq, base, states })
    }

    /// A message from the host (the ones about the map; the rest are the window's).
    pub fn received(&mut self, m: Msg, doc: &mut Document, history: &mut History) -> Vec<Note> {
        let mut notes = Vec::new();
        match m {
            Msg::Applied { seq: Some(seq), states, .. } => {
                if self.flight.as_ref().map(|f| f.0) == Some(seq) {
                    self.flight = None;
                }
                for s in states {
                    self.known.insert(s.part, s.version);
                }
            }
            Msg::Applied { author, seq: None, states } => {
                let replaced = states.iter().filter(|s| self.dirty.contains(&s.part)).count();
                if replaced > 0 {
                    notes.push(Note::Replaced { by: author, parts: replaced });
                }
                self.put(states, doc, history, &mut notes);
            }
            Msg::Rejected { seq, why, states } => {
                // (put back while the edit is still the one on its way: what it held is kept
                // for recovery first)
                notes.push(Note::TurnedDown { tiles: states.iter().filter(|s| matches!(s.part, Part::Tile(..))).count(), why });
                self.put(states, doc, history, &mut notes);
                if self.flight.as_ref().map(|f| f.0) == Some(seq) {
                    self.flight = None;
                }
            }
            Msg::Ids(a, b) => {
                self.spare = Some((a, b));
                self.asked = false;
            }
            _ => {}
        }
        notes
    }

    /// The host's state of these parts into the map.
    fn put(&mut self, mut states: Vec<State>, doc: &mut Document, history: &mut History, notes: &mut Vec<Note>) {
        let conflicts: Vec<Part> = states.iter().map(|s| s.part).filter(|p| {
            self.dirty.contains(p) || self.flight.as_ref().map(|(_, parts)| parts.contains(p)).unwrap_or(false)
        }).collect();
        if !conflicts.is_empty() {
            match super::recovery::keep(doc, &conflicts) {
                Ok(path) => notes.push(Note::Recovered(path)),
                Err(error) => {
                    notes.push(Note::RecoveryFailed(error.to_string()));
                    return;
                }
            }
        }
        // (the global settings first: the tiles' masks are made to fit its ground layers)
        states.sort_by_key(|s| s.part != Part::Global);
        let mut forget = Changed::default();
        for s in states {
            let ok = match s.part {
                Part::Tile(x, y) => {
                    forget.tiles.insert((x, y));
                    codec::apply_tile(doc, (x, y), s.data.as_deref())
                }
                Part::Global => {
                    forget.global = true;
                    s.data.as_deref().map(|b| codec::apply_global(doc, b)).unwrap_or(false)
                }
                Part::Groups => {
                    forget.groups = true;
                    s.data.as_deref().map(|b| codec::apply_groups(doc, b)).unwrap_or(false)
                }
                // (not passed on in this version)
                Part::Timetable | Part::Traffic | Part::Descriptions => true,
            };
            if !ok {
                notes.push(Note::Unreadable(s.part));
            }
            self.known.insert(s.part, s.version);
            self.dirty.remove(&s.part);
        }
        history.forget(&forget);
    }

    /// The block of IDCodes kept going: the next one taken when this one is used up, a new
    /// one asked for when half of it is (the message to send).
    pub fn keep_ids(&mut self, doc: &mut Document) -> Option<Msg> {
        let next = &mut doc.global.next_id_code;
        if *next < self.ids.0 {
            *next = self.ids.0;
        }
        if *next >= self.ids.1 {
            match self.spare.take() {
                Some(b) => {
                    self.ids = b;
                    *next = b.0;
                }
                // (cannot be: a block lasts long after the next is asked for)
                None => log::error!("multiplayer: the IDCodes of this editor ran out before the next block came"),
            }
        }
        if self.spare.is_none() && !self.asked && *next >= self.ids.0 + (self.ids.1 - self.ids.0) / 2 {
            self.asked = true;
            return Some(Msg::NeedIds);
        }
        None
    }
}

/// A part's bytes as the map has it now (None: a tile not there).
pub fn bytes_of(doc: &Document, part: Part) -> Option<Vec<u8>> {
    match part {
        Part::Tile(x, y) => doc.tiles.get(&(x, y)).map(codec::tile_bytes),
        Part::Global => Some(codec::global_bytes(doc)),
        Part::Groups => Some(codec::groups_bytes(doc)),
        Part::Timetable | Part::Traffic | Part::Descriptions => None,
    }
}

/// Every part of the map, for a host to start with.
pub fn all_states(doc: &Document) -> Vec<State> {
    let mut parts: Vec<Part> = doc.tiles.keys().map(|k| Part::Tile(k.0, k.1)).collect();
    parts.push(Part::Global);
    parts.push(Part::Groups);
    parts.into_iter().map(|part| State { part, version: 1, data: bytes_of(doc, part) }).collect()
}
