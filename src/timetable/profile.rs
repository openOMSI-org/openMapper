//! A trip's time profiles (`[profile]` of its `.ttp`): each its time from the first stop to
//! the last and the times fixed at stops between, so the time from one stop to the next can
//! be set. A tour's departure names the profile it runs to by its place in the list.

use omsi_timetable::TripProfile;

use super::{stop_times, Timetable};

/// The time from stop `i - 1` to stop `i` of profile `p` set to `m` minutes (`legs`: the
/// metres between the stops): the stop before keeps its time, stop `i` and every time after
/// it move by the difference, the trip's time with them. The other stops' legs stay as they
/// were.
pub fn set_leg(p: &mut TripProfile, n: usize, legs: &[f64], i: usize, m: f32) {
    if i == 0 || i >= n {
        return;
    }
    let times = stop_times(n, legs, p);
    let shown = |k: usize| if k + 1 == n { times[k].0 } else { times[k].1 };
    let start = times[i - 1].1;
    let m = m.max(0.0);
    let delta = start + m - shown(i);
    if delta.abs() < 1e-4 {
        return;
    }
    // (the stop before fixed where it is, so that the leg is the one set)
    if i - 1 > 0 && !times[i - 1].2 {
        super::set_stop_time(p, n, i - 1, Some(round(times[i - 1].1)));
    }
    for k in i..n {
        if times[k].2 || (k == i && k + 1 < n) {
            super::set_stop_time(p, n, k, Some(round(shown(k) + delta)));
        }
    }
    p.factor = round(times[n - 1].0 + delta).max(0.1);
    // (the last stop's time is the trip's)
    if let Some(a) = p.man_arr_time.iter_mut().find(|a| a.0 as usize + 1 == n) {
        a.1 = p.factor;
    }
}

/// The trip's time of profile `p` set to `m` minutes: the times fixed at the stops are kept,
/// the last stop's with it.
pub fn set_total(p: &mut TripProfile, n: usize, m: f32) {
    p.factor = m;
    let last = n as i32 - 1;
    for a in p.man_arr_time.iter_mut().filter(|a| a.0 == last) {
        a.1 = m;
    }
}

/// Minutes kept to a hundredth (the file keeps three decimals).
fn round(m: f32) -> f32 {
    (m * 100.0).round() / 100.0
}

impl Timetable {
    /// A new profile of the trip, a copy of profile `from` under a new name: its place.
    pub fn add_profile(&mut self, trip: &str, from: usize) -> Option<usize> {
        let t = self.trip_mut(trip)?;
        let mut p = t.profiles.get(from).cloned().unwrap_or_else(|| TripProfile { name: "standard".into(), factor: 10.0, ..Default::default() });
        let base = p.name.trim().to_string();
        p.name = (2..).map(|k| format!("{base} {k}")).find(|n| !t.profiles.iter().any(|x| x.name.trim().eq_ignore_ascii_case(n))).unwrap();
        t.profiles.push(p);
        let i = t.profiles.len() - 1;
        self.touch_trip(trip);
        Some(i)
    }

    /// Take the trip's profile `i` away (not its last one): the departures that ran to it run
    /// to its first, those to a later one keep theirs.
    pub fn remove_profile(&mut self, trip: &str, i: usize) {
        let Some(t) = self.trip_mut(trip) else { return };
        if t.profiles.len() < 2 || i >= t.profiles.len() {
            return;
        }
        t.profiles.remove(i);
        self.touch_trip(trip);
        let mut touched = Vec::new();
        for l in &mut self.lines {
            let mut hit = false;
            for x in l.tours.iter_mut().flat_map(|t| t.trips.iter_mut()).filter(|x| x.trip.eq_ignore_ascii_case(trip)) {
                let k = x.profile.max(0) as usize;
                if k == i {
                    x.profile = 0;
                    hit = true;
                } else if k > i {
                    x.profile -= 1;
                    hit = true;
                }
            }
            if hit {
                touched.push(l.name.clone());
            }
        }
        for l in touched {
            self.touch_line(&l);
        }
    }
}
