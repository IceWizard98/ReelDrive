//! How far through something the user got, and what to play next.
//!
//! Pure: the clock arrives as an argument and nothing here opens a file. The
//! key of a mark is the video's path relative to the media root, which is the
//! only identity an episode has anywhere in this app — there are no ids below
//! the content folder.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::core::model::{ContentBody, ContentDetail};
use crate::core::paths;

/// Bumped by hand when the shape below changes, and only then.
///
/// Unlike the scan cache this is **not** tied to the build number: a cache may
/// be thrown away on every release, a watch history may not. A version this
/// build does not understand is set aside, never overwritten.
pub const PROGRESS_VERSION: u32 = 1;

/// Below this, nothing is remembered. Opening the wrong file and closing it
/// again is not "having started" something, and a library where every mistaken
/// double-click leaves a half-watched title is worse than one that forgets.
pub const MIN_SECONDS: f64 = 15.0;

/// The same idea for anything too short for `MIN_SECONDS` to be a fraction of.
///
/// Fifteen seconds is a moment of a film and the whole of a clip: as a fixed
/// number it is a wall in front of anything shorter, which can then only ever
/// record "finished" or nothing at all. Whichever of the two is smaller applies,
/// so a half-hour episode keeps the fifteen seconds it was written for and a
/// four-second file gets a floor its own size.
pub const MIN_FRACTION: f64 = 0.05;

/// At or past this fraction the film is finished. The last minutes are credits
/// often enough that waiting for the true end would leave everything one press
/// short of done for ever.
pub const DONE_FRACTION: f64 = 0.95;

/// Credits a viewer is allowed to walk out on, however short the thing is.
///
/// The fraction above is the wrong shape on its own, for the same reason a
/// fixed floor was: closing titles run for a *length of time*, not for a
/// percentage. A twentieth of a twenty-minute episode is one minute, which is
/// shorter than the outro of every anime ever made — so the viewer stops when
/// the song starts and the episode stays unwatched for ever. Two minutes is the
/// shortest allowance that covers that.
pub const CREDITS_SECONDS: f64 = 120.0;

/// …but never call something finished before this much of it has been seen.
///
/// Two minutes is longer than a short file, and without a floor under the
/// allowance every clip would be born finished — the fixtures on a developer's
/// stick would all read "watched" before playing.
pub const MIN_DONE_FRACTION: f64 = 0.80;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Mark {
    /// Seconds into the film, on the film's own clock.
    pub seconds: f64,
    /// Length of the film, kept so the home screen can draw a bar without
    /// probing every file it shows — a probe costs a process launch.
    pub duration: f64,
    pub done: bool,
    /// Unix seconds, for ordering "continue watching". Written by the caller,
    /// which is what keeps this module free of a clock.
    pub at: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProgressData {
    pub version: u32,
    pub items: BTreeMap<String, Mark>,
}

impl Default for ProgressData {
    fn default() -> Self {
        Self {
            version: PROGRESS_VERSION,
            items: BTreeMap::new(),
        }
    }
}

/// What the play button should do next for one content.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UpNext {
    /// Video file relative to the media root.
    pub file: String,
    pub subtitles: Vec<String>,
    /// `None` for a film, which has no season or episode to name.
    pub season: Option<u32>,
    pub episode: Option<u32>,
    pub title: String,
    /// Where to start it. Zero for something never opened, or finished.
    pub seconds: f64,
    /// True when nothing of this content has ever been played, so the button
    /// can say "Play" rather than "Resume".
    pub fresh: bool,
}

impl ProgressData {
    /// Drop everything a hand-edited file could carry that the app must not
    /// believe.
    ///
    /// This file lives on the user's stick, so it is untrusted input in exactly
    /// the way the scan cache is — and the key is used as a path. The syntactic
    /// rules are `paths`' own, so the two cannot drift apart.
    pub fn sanitized(mut self) -> Self {
        self.items
            .retain(|key, mark| paths::is_safe_relative(key) && mark.is_believable());
        self.version = PROGRESS_VERSION;
        self
    }

    /// Remember where `path` got to, returning the mark that was stored.
    ///
    /// `None` means nothing was written: a few seconds of a film is not a
    /// position worth keeping, and it must not overwrite the real one already
    /// there — restarting something and giving up ten seconds in would
    /// otherwise throw away where you actually were.
    pub fn record(&mut self, path: &str, seconds: f64, duration: f64, now: u64) -> Option<Mark> {
        if !paths::is_safe_relative(path) {
            return None;
        }
        let reported = Mark {
            seconds,
            duration,
            done: false,
            at: now,
        };
        if !reported.is_believable() {
            return None;
        }
        let done = seconds >= finished_at(duration);
        if seconds < MIN_SECONDS.min(duration * MIN_FRACTION) && !done {
            return None;
        }
        let stored = Mark {
            // A finished film resumes from the start, so there is no reason to
            // keep the second it happened to end on.
            seconds: if done { 0.0 } else { seconds },
            duration,
            done,
            at: now,
        };
        self.items.insert(path.to_string(), stored);
        Some(stored)
    }

    /// Put a file at the head of the history without claiming any of it was
    /// watched. Used when an episode ends and the next one takes its place, so
    /// the series does not drop out of "continue watching" between them.
    pub fn touch(&mut self, path: &str, duration: f64, now: u64) -> Option<Mark> {
        if !paths::is_safe_relative(path) || self.items.contains_key(path) {
            return None;
        }
        let mark = Mark {
            seconds: 0.0,
            duration,
            done: false,
            at: now,
        };
        if !mark.is_believable() {
            return None;
        }
        self.items.insert(path.to_string(), mark);
        Some(mark)
    }

    /// Second to start `path` at. Zero for anything unknown or finished.
    pub fn resume_seconds(&self, path: &str) -> f64 {
        match self.items.get(path) {
            Some(mark) if !mark.done => mark.seconds,
            _ => 0.0,
        }
    }

    /// The most recently touched mark belonging to `id`, which is the content
    /// folder name — so the match is on the `{id}/` prefix and `Scrubs` cannot
    /// collect the marks of `Scrubs 2`.
    pub fn latest(&self, id: &str) -> Option<(&String, &Mark)> {
        let prefix = format!("{id}/");
        self.items
            .iter()
            .filter(|(key, _)| key.starts_with(&prefix))
            // Ties broken by key so the answer never depends on map order.
            .max_by(|(a_key, a), (b_key, b)| a.at.cmp(&b.at).then_with(|| a_key.cmp(b_key)))
    }
}

/// The second from which something counts as watched to the end.
///
/// Whichever of the two allowances is kinder, floored so a short file cannot be
/// finished before most of it has gone by. An episode is decided by the two
/// minutes, a film by the fraction its greater length earns it, and the two
/// meet at forty minutes — 5% of 40:00 is exactly the two minutes — which is
/// why nothing above that changed when this replaced a bare fraction.
///
/// ```text
///   4.2s  ->  3.36  (80%, the floor, up to 10:00)
///  20:00  ->  18:00 (2:00 of credits)
///  40:00  ->  38:00 (the two meet: 5% is 2:00)
///  45:00  ->  42:45 (fraction wins from 40:00 up)
///   2:00h ->  1:54  (6:00 of credits)
/// ```
pub fn finished_at(duration: f64) -> f64 {
    (duration - CREDITS_SECONDS)
        .min(duration * DONE_FRACTION)
        .max(duration * MIN_DONE_FRACTION)
}

impl Mark {
    /// Numbers a player could actually have produced. JSON carries no NaN, but
    /// a hand-edited file carries anything, and a NaN reaches the screen as a
    /// bar of no length and a resume to nowhere.
    fn is_believable(&self) -> bool {
        self.seconds.is_finite()
            && self.duration.is_finite()
            && self.seconds >= 0.0
            && self.duration > 0.0
            && self.seconds <= self.duration
    }

    /// How much of it has been watched, 0 to 1.
    pub fn fraction(&self) -> f64 {
        if self.done {
            return 1.0;
        }
        (self.seconds / self.duration).clamp(0.0, 1.0)
    }
}

/// The episode to play now: the first one not finished, seasons in order.
///
/// Crossing a season boundary needs no special case — the seasons are walked as
/// one list, which is the whole reason this is derived rather than stored. A
/// remembered "last episode" pointer would have to be corrected every time a
/// folder is renamed or an episode is added, and would quietly lie until it
/// was.
///
/// Extras (season 0) are skipped unless they are all there is: they are not
/// where a series continues, and a stick with a `Specials` folder would
/// otherwise never advance past it.
pub fn up_next(detail: &ContentDetail, data: &ProgressData) -> Option<UpNext> {
    match &detail.body {
        ContentBody::Movie { file, subtitles } => Some(UpNext {
            file: file.clone(),
            subtitles: subtitles.clone(),
            season: None,
            episode: None,
            title: detail.summary.title.clone(),
            seconds: data.resume_seconds(file),
            fresh: !data.items.contains_key(file),
        }),
        ContentBody::Series { seasons } => {
            let mut ordered: Vec<_> = seasons.iter().collect();
            ordered.sort_by_key(|season| season.number);
            let only_extras = ordered.iter().all(|season| season.number == 0);

            let fresh = !ordered
                .iter()
                .flat_map(|season| &season.episodes)
                .any(|episode| data.items.contains_key(&episode.file));

            ordered
                .iter()
                .filter(|season| only_extras || season.number != 0)
                .flat_map(|season| season.episodes.iter().map(move |episode| (season, episode)))
                .find(|(_, episode)| !data.items.get(&episode.file).is_some_and(|m| m.done))
                .map(|(season, episode)| UpNext {
                    file: episode.file.clone(),
                    subtitles: episode.subtitles.clone(),
                    season: Some(season.number),
                    episode: Some(episode.number),
                    title: episode.title.clone(),
                    seconds: data.resume_seconds(&episode.file),
                    fresh,
                })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::model::{ContentKind, ContentSummary, Episode, Season};

    const NOW: u64 = 1_765_000_000;

    fn data() -> ProgressData {
        ProgressData::default()
    }

    fn series(seasons: Vec<Season>) -> ContentDetail {
        ContentDetail {
            summary: ContentSummary {
                id: "Scrubs".into(),
                title: "Scrubs".into(),
                year: None,
                cover: None,
                kind: ContentKind::Series,
            },
            body: ContentBody::Series { seasons },
        }
    }

    fn season(number: u32, files: &[&str]) -> Season {
        Season {
            number,
            title: format!("Season {number}"),
            episodes: files
                .iter()
                .enumerate()
                .map(|(i, file)| Episode {
                    number: i as u32 + 1,
                    title: format!("Episode {}", i + 1),
                    file: (*file).to_string(),
                    subtitles: Vec::new(),
                })
                .collect(),
        }
    }

    fn movie(file: &str) -> ContentDetail {
        ContentDetail {
            summary: ContentSummary {
                id: "Inception (2010)".into(),
                title: "Inception".into(),
                year: Some(2010),
                cover: None,
                kind: ContentKind::Movie,
            },
            body: ContentBody::Movie {
                file: file.into(),
                subtitles: Vec::new(),
            },
        }
    }

    #[test]
    fn a_glance_at_a_film_is_not_a_position_worth_keeping() {
        // The cost of remembering too eagerly is a library where every wrong
        // double-click leaves a half-watched title behind for ever.
        let mut data = data();
        assert_eq!(data.record("Film/film.mkv", 14.9, 6000.0, NOW), None);
        assert!(data.items.is_empty());
    }

    #[test]
    fn a_position_past_the_floor_is_kept() {
        let mut data = data();
        let mark = data.record("Film/film.mkv", 15.0, 6000.0, NOW).unwrap();
        assert_eq!(mark.seconds, 15.0);
        assert!(!mark.done);
        assert_eq!(mark.at, NOW);
        assert_eq!(data.items.len(), 1);
    }

    #[test]
    fn the_last_of_a_film_counts_as_all_of_it() {
        // Credits are long enough that a strict end would leave everything one
        // press short of finished.
        let mut data = data();
        let mark = data.record("Film/film.mkv", 5700.0, 6000.0, NOW).unwrap();
        assert!(mark.done);
        assert_eq!(mark.seconds, 0.0, "a finished film restarts from the start");
    }

    #[test]
    fn an_episode_is_finished_once_only_its_closing_credits_are_left() {
        // Credits run for a length of time, not for a percentage. A twentieth
        // of a twenty-minute episode is one minute, which is shorter than the
        // outro of every anime ever made: the viewer stops when the song
        // starts and the episode stays unwatched for ever.
        let mut data = data();
        let twenty_minutes = 1200.0;

        assert!(
            data.record("Anime/e1.mkv", 1080.0, twenty_minutes, NOW)
                .unwrap()
                .done,
            "two minutes from the end is the end"
        );
        assert!(
            !data
                .record("Anime/e2.mkv", 1079.0, twenty_minutes, NOW)
                .unwrap()
                .done,
            "a second earlier is not"
        );
    }

    #[test]
    fn a_feature_film_keeps_the_fraction_its_longer_credits_earn() {
        // Two minutes would be a stricter rule for a film, not a looser one:
        // its credits are longer than an episode's, so the fraction is what
        // should win once the film is long enough for it to mean more.
        let mut data = data();
        let two_hours = 7200.0;

        assert!(
            data.record("Film/a.mkv", 6840.0, two_hours, NOW)
                .unwrap()
                .done
        );
        assert!(
            !data
                .record("Film/b.mkv", 6839.0, two_hours, NOW)
                .unwrap()
                .done
        );
    }

    #[test]
    fn a_short_file_is_never_finished_before_four_fifths_of_it() {
        // The two-minute allowance is longer than the whole file here. Without
        // a floor under it every clip would be born finished, and the fixtures
        // on a developer's stick would all read "watched" before playing.
        // Either side of four fifths rather than exactly on it: `4.2 * 0.8` is
        // 3.3600000000000003, so a literal `3.36` would sit below its own
        // threshold and the test would be pinning a rounding artefact instead
        // of the rule.
        let mut data = data();
        assert!(data.record("Clip/a.mkv", 3.4, 4.2, NOW).unwrap().done);
        assert!(!data.record("Clip/b.mkv", 3.3, 4.2, NOW).unwrap().done);
        assert_eq!(finished_at(4.2), 4.2 * MIN_DONE_FRACTION);
    }

    #[test]
    fn the_three_allowances_hand_over_where_the_documentation_says_they_do() {
        // Both boundaries are stated in the README and in `finished_at`'s own
        // comment, and until this test they were prose nobody checked. Two
        // minutes is a fifth of ten minutes and a twentieth of forty, which is
        // what puts the handovers exactly there.
        //
        // Each is asserted twice, against *both* formulas, and that pairing is
        // the whole of the test: `finished_at(600) == 600 * MIN_DONE_FRACTION`
        // alone would restate the implementation and survive the constant
        // changing, but nothing except 0.80 also makes it `600 - 120`.
        assert_eq!(finished_at(600.0), 600.0 * MIN_DONE_FRACTION, "10:00");
        assert_eq!(finished_at(600.0), 600.0 - CREDITS_SECONDS);
        assert_eq!(finished_at(2400.0), 2400.0 - CREDITS_SECONDS, "40:00");
        assert_eq!(finished_at(2400.0), 2400.0 * DONE_FRACTION);

        // Either side of each, so the handover is pinned and not just the seam.
        assert_eq!(finished_at(300.0), 300.0 * MIN_DONE_FRACTION);
        assert_eq!(finished_at(1200.0), 1200.0 - CREDITS_SECONDS);
        assert_eq!(finished_at(7200.0), 7200.0 * DONE_FRACTION);
    }

    #[test]
    fn nothing_longer_than_the_crossover_moved_when_the_bare_fraction_went() {
        // Films were already right, and the change had to leave them alone.
        //
        // This one restates the fraction, so it cannot pin its value — a
        // `finished_at` that was nothing but `d * DONE_FRACTION` would pass it.
        // What it pins is the *extent* of that regime: 40 minutes is the
        // crossover itself and the rest are past it, so a rewrite that put the
        // handover anywhere else is caught here and nowhere else in this file.
        for minutes in [40, 45, 60, 90, 120, 180, 600] {
            let d = f64::from(minutes) * 60.0;
            assert_eq!(
                finished_at(d),
                d * DONE_FRACTION,
                "{minutes} minutes should be untouched"
            );
        }
    }

    #[test]
    fn a_film_is_never_finished_before_it_has_started() {
        // The floor under "finished" and the floor under "remembered at all"
        // are separate numbers, and a duration where they crossed would leave
        // a file finished before it counted as begun — with no second of it
        // that could be stored as a partial position at all.
        //
        // Asked of `record` rather than of the two formulas: recomputing the
        // floor here would restate the line under test, and would stay green
        // through that floor changing shape rather than value.
        for d in [
            0.001, 0.5, 4.2, 15.0, 60.0, 300.0, 600.0, 1200.0, 2400.0, 7200.0, 36_000.0,
        ] {
            let done = finished_at(d);
            assert!(done < d, "d={d}: done {done} not below the end");

            let short_of_it = done * 0.99;
            let mark = data()
                .record("Clip/a.mkv", short_of_it, d, NOW)
                .unwrap_or_else(|| panic!("d={d}: {short_of_it} is below the floor as well"));
            assert!(!mark.done, "d={d}: {short_of_it} is not the end of it");
        }
    }

    #[test]
    fn a_short_file_is_remembered_in_proportion_to_its_own_length() {
        // Fifteen seconds is a sensible fraction of a film and the whole of a
        // clip. A fixed floor is a wall in front of anything short: nothing
        // below it can ever be remembered, so a four-second file only ever
        // records "finished" or nothing at all.
        let mut data = data();
        let mark = data.record("Clip/clip.mkv", 0.3, 4.2, NOW).unwrap();
        assert_eq!(mark.seconds, 0.3);
        assert!(!mark.done);
        // Still a floor, just its own: a glance at it is still a glance.
        assert_eq!(data.record("Clip/other.mkv", 0.1, 4.2, NOW), None);
    }

    #[test]
    fn a_feature_length_film_keeps_the_fifteen_second_floor() {
        // The proportional rule must not loosen the case it was written for: a
        // twentieth of a half-hour episode is ninety seconds, and taking that
        // as the floor would forget the first minute and a half of everything.
        let mut data = data();
        assert_eq!(data.record("Show/e1.mkv", 14.9, 1800.0, NOW), None);
        assert_eq!(
            data.record("Show/e1.mkv", 15.0, 1800.0, NOW)
                .map(|m| m.seconds),
            Some(15.0)
        );
    }

    #[test]
    fn the_floor_never_grows_past_fifteen_seconds_however_long_the_film() {
        // A ten-hour recording would otherwise ask for half an hour before it
        // remembered anything.
        let mut data = data();
        assert_eq!(
            data.record("Long/rec.mkv", 15.0, 36_000.0, NOW)
                .map(|m| m.seconds),
            Some(15.0)
        );
    }

    #[test]
    fn a_clip_stopped_just_short_of_its_end_counts_as_finished() {
        // The completion rule is a fraction, so it means the same thing at ten
        // seconds as at two hours — a clip left in its last tenth is watched,
        // not abandoned.
        let mut data = data();
        let mark = data.record("Clip/clip.mkv", 9.8, 10.0, NOW).unwrap();
        assert!(mark.done);
    }

    #[test]
    fn giving_up_seconds_into_a_replay_does_not_erase_where_you_were() {
        // The floor has to protect what is already stored, not only refuse to
        // write: reopening something and closing it again is the ordinary way
        // to lose a position.
        let mut data = data();
        data.record("Film/film.mkv", 3000.0, 6000.0, NOW).unwrap();
        assert_eq!(data.record("Film/film.mkv", 4.0, 6000.0, NOW + 60), None);
        assert_eq!(data.items["Film/film.mkv"].seconds, 3000.0);
    }

    #[test]
    fn a_later_position_replaces_the_one_before_it() {
        let mut data = data();
        data.record("Film/film.mkv", 100.0, 6000.0, NOW).unwrap();
        data.record("Film/film.mkv", 900.0, 6000.0, NOW + 800)
            .unwrap();
        assert_eq!(data.items["Film/film.mkv"].seconds, 900.0);
        assert_eq!(data.items["Film/film.mkv"].at, NOW + 800);
    }

    #[test]
    fn a_position_beyond_the_film_is_refused() {
        // A stream that restarted at an offset reports its own clock; adding it
        // to the wrong base is how a position past the end arrives.
        let mut data = data();
        assert_eq!(data.record("Film/film.mkv", 9000.0, 6000.0, NOW), None);
        assert_eq!(data.record("Film/film.mkv", 100.0, 0.0, NOW), None);
        assert_eq!(data.record("Film/film.mkv", -1.0, 6000.0, NOW), None);
        assert!(data.items.is_empty());
    }

    #[test]
    fn a_path_that_could_leave_the_media_folder_is_never_recorded() {
        let mut data = data();
        for path in [
            "../secrets/id_rsa",
            "/etc/passwd",
            "a\\b.mkv",
            "a\0b.mkv",
            "",
        ] {
            assert_eq!(data.record(path, 100.0, 6000.0, NOW), None, "path: {path}");
        }
        assert!(data.items.is_empty());
    }

    #[test]
    fn a_hand_edited_file_cannot_smuggle_in_a_path_or_a_number() {
        // Same standing as the scan cache: it sits on the user's stick, anyone
        // can open it, and the key is used as a path.
        let mut items = BTreeMap::new();
        for key in [
            "../../etc/passwd",
            "/etc/passwd",
            "a\\b.mkv",
            "Film/film.mkv",
        ] {
            items.insert(
                key.to_string(),
                Mark {
                    seconds: 10.0,
                    duration: 100.0,
                    done: false,
                    at: NOW,
                },
            );
        }
        items.insert(
            "Bad/nan.mkv".into(),
            Mark {
                seconds: f64::NAN,
                duration: 100.0,
                done: false,
                at: NOW,
            },
        );
        items.insert(
            "Bad/forever.mkv".into(),
            Mark {
                seconds: 10.0,
                duration: f64::INFINITY,
                done: false,
                at: NOW,
            },
        );

        let clean = ProgressData { version: 99, items }.sanitized();

        assert_eq!(clean.items.keys().collect::<Vec<_>>(), ["Film/film.mkv"]);
        assert_eq!(clean.version, PROGRESS_VERSION);
    }

    #[test]
    fn a_finished_film_starts_again_from_the_beginning() {
        let mut data = data();
        data.record("Film/film.mkv", 5900.0, 6000.0, NOW).unwrap();
        assert_eq!(data.resume_seconds("Film/film.mkv"), 0.0);
        assert_eq!(data.resume_seconds("Never/opened.mkv"), 0.0);
    }

    #[test]
    fn a_finished_mark_carrying_a_position_still_starts_from_the_beginning() {
        // `record` never writes that pair, so the guard in `resume_seconds`
        // only earns its place against the file on the stick — which anyone
        // can edit, and which nothing else stops from saying both at once.
        let mut data = data();
        data.items.insert(
            "Film/film.mkv".into(),
            Mark {
                seconds: 5900.0,
                duration: 6000.0,
                done: true,
                at: NOW,
            },
        );
        assert_eq!(data.resume_seconds("Film/film.mkv"), 0.0);
    }

    #[test]
    fn the_latest_mark_belongs_to_the_folder_it_names_and_no_other() {
        // Prefix matching on the bare id would give `Scrubs` the history of
        // `Scrubs 2`, which is a different show on the same stick.
        let mut data = data();
        data.record("Scrubs/S01/e1.mkv", 100.0, 1000.0, NOW)
            .unwrap();
        data.record("Scrubs/S01/e2.mkv", 200.0, 1000.0, NOW + 10)
            .unwrap();
        data.record("Scrubs 2/S01/e1.mkv", 300.0, 1000.0, NOW + 20)
            .unwrap();

        let (path, mark) = data.latest("Scrubs").unwrap();
        assert_eq!(path, "Scrubs/S01/e2.mkv");
        assert_eq!(mark.seconds, 200.0);
        assert!(data.latest("Nothing").is_none());
    }

    #[test]
    fn a_film_is_its_own_next_thing_to_play() {
        let mut data = data();
        let detail = movie("Inception (2010)/Inception.mkv");
        let first = up_next(&detail, &data).unwrap();
        assert_eq!(first.file, "Inception (2010)/Inception.mkv");
        assert_eq!(first.seconds, 0.0);
        assert!(first.fresh);
        assert_eq!(first.season, None);

        data.record("Inception (2010)/Inception.mkv", 900.0, 6000.0, NOW)
            .unwrap();
        let again = up_next(&detail, &data).unwrap();
        assert_eq!(again.seconds, 900.0);
        assert!(!again.fresh);
    }

    #[test]
    fn a_series_nobody_has_opened_starts_at_its_first_episode() {
        let detail = series(vec![
            season(1, &["Scrubs/S01/e1.mkv", "Scrubs/S01/e2.mkv"]),
            season(2, &["Scrubs/S02/e1.mkv"]),
        ]);
        let next = up_next(&detail, &data()).unwrap();
        assert_eq!(next.file, "Scrubs/S01/e1.mkv");
        assert_eq!(next.season, Some(1));
        assert_eq!(next.episode, Some(1));
        assert!(next.fresh);
    }

    #[test]
    fn a_part_watched_episode_is_the_one_to_carry_on_with() {
        let detail = series(vec![season(1, &["Scrubs/S01/e1.mkv", "Scrubs/S01/e2.mkv"])]);
        let mut data = data();
        data.record("Scrubs/S01/e1.mkv", 400.0, 1300.0, NOW)
            .unwrap();
        let next = up_next(&detail, &data).unwrap();
        assert_eq!(next.file, "Scrubs/S01/e1.mkv");
        assert_eq!(next.seconds, 400.0);
        assert!(!next.fresh);
    }

    #[test]
    fn finishing_an_episode_moves_to_the_one_after_it() {
        let detail = series(vec![season(1, &["Scrubs/S01/e1.mkv", "Scrubs/S01/e2.mkv"])]);
        let mut data = data();
        data.record("Scrubs/S01/e1.mkv", 1290.0, 1300.0, NOW)
            .unwrap();
        assert_eq!(up_next(&detail, &data).unwrap().file, "Scrubs/S01/e2.mkv");
    }

    #[test]
    fn the_end_of_a_season_leads_into_the_next_one() {
        // The reason this is derived rather than remembered: no pointer has to
        // know that a season ended, because the seasons are one list.
        let detail = series(vec![
            season(1, &["Scrubs/S01/e1.mkv", "Scrubs/S01/e2.mkv"]),
            season(2, &["Scrubs/S02/e1.mkv"]),
        ]);
        let mut data = data();
        data.record("Scrubs/S01/e1.mkv", 1290.0, 1300.0, NOW)
            .unwrap();
        data.record("Scrubs/S01/e2.mkv", 1290.0, 1300.0, NOW + 1)
            .unwrap();

        let next = up_next(&detail, &data).unwrap();
        assert_eq!(next.file, "Scrubs/S02/e1.mkv");
        assert_eq!(next.season, Some(2));
        assert_eq!(next.seconds, 0.0);
        assert!(
            !next.fresh,
            "the series has been watched, this episode has not"
        );
    }

    #[test]
    fn seasons_are_walked_in_number_order_whatever_order_they_arrive_in() {
        let detail = series(vec![
            season(2, &["Scrubs/S02/e1.mkv"]),
            season(1, &["Scrubs/S01/e1.mkv"]),
        ]);
        assert_eq!(up_next(&detail, &data()).unwrap().file, "Scrubs/S01/e1.mkv");
    }

    #[test]
    fn extras_are_not_where_a_series_carries_on() {
        // A stick with a `Specials` folder would otherwise never advance past
        // it: season 0 sorts first and nothing in it is ever the next episode.
        let detail = series(vec![
            season(0, &["Scrubs/Specials/x.mkv"]),
            season(1, &["Scrubs/S01/e1.mkv"]),
        ]);
        assert_eq!(up_next(&detail, &data()).unwrap().file, "Scrubs/S01/e1.mkv");
    }

    #[test]
    fn extras_are_offered_when_they_are_the_only_thing_there() {
        // Skipping them unconditionally left the play button with nothing to
        // do on a folder that holds only specials.
        let detail = series(vec![season(0, &["Scrubs/Specials/x.mkv"])]);
        assert_eq!(
            up_next(&detail, &data()).unwrap().file,
            "Scrubs/Specials/x.mkv"
        );
    }

    #[test]
    fn a_series_watched_to_the_end_has_nothing_left_to_resume() {
        let detail = series(vec![season(1, &["Scrubs/S01/e1.mkv"])]);
        let mut data = data();
        data.record("Scrubs/S01/e1.mkv", 1290.0, 1300.0, NOW)
            .unwrap();
        assert_eq!(up_next(&detail, &data), None);
    }

    #[test]
    fn a_season_with_no_playable_files_is_stepped_over() {
        let detail = series(vec![season(1, &[]), season(2, &["Scrubs/S02/e1.mkv"])]);
        assert_eq!(up_next(&detail, &data()).unwrap().file, "Scrubs/S02/e1.mkv");
    }

    #[test]
    fn taking_up_the_next_episode_dates_it_without_claiming_it_was_watched() {
        // Without this a series leaves "continue watching" the moment an
        // episode ends, which is the exact moment it is most being watched.
        let mut data = data();
        let mark = data.touch("Scrubs/S01/e2.mkv", 1300.0, NOW).unwrap();
        assert_eq!(mark.seconds, 0.0);
        assert_eq!(mark.at, NOW);
        assert_eq!(data.latest("Scrubs").unwrap().0, "Scrubs/S01/e2.mkv");
    }

    #[test]
    fn taking_up_an_episode_already_started_leaves_its_position_alone() {
        let mut data = data();
        data.record("Scrubs/S01/e2.mkv", 600.0, 1300.0, NOW)
            .unwrap();
        assert_eq!(data.touch("Scrubs/S01/e2.mkv", 1300.0, NOW + 99), None);
        assert_eq!(data.items["Scrubs/S01/e2.mkv"].seconds, 600.0);
    }

    #[test]
    fn a_finished_mark_reads_as_a_full_bar_whatever_second_it_stopped_on() {
        let done = Mark {
            seconds: 0.0,
            duration: 1300.0,
            done: true,
            at: NOW,
        };
        assert_eq!(done.fraction(), 1.0);
        let half = Mark {
            seconds: 650.0,
            duration: 1300.0,
            done: false,
            at: NOW,
        };
        assert_eq!(half.fraction(), 0.5);
    }
}
