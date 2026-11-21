//! What a browser plays, a YouTube video in Chrome for one, as macOS itself has it: the
//! title, the channel, where it is and whether it plays, the same that Control Center shows.
//! macOS tells that only to its own programs, so `osascript` asks on Buntini's behalf. A
//! watcher keeps running and writes a line each time it changes, while a browser is switched
//! on in Settings → Apps; only those browsers show. Nothing is asked of the browser, so no
//! macOS permission is needed and none is asked for. The card's controls reach the
//! browser as a media key would, sent from Buntini itself (`native::command`), which macOS
//! allows any app.
use std::process::Stdio;

use serde::Deserialize;
use tokio::io::{AsyncBufReadExt, BufReader, Lines};
use tokio::process::{Child, ChildStdout, Command};

use super::player::{unix_now, Player, Read, Reading};

/// Seconds between 1970 and 2001, where macOS counts its dates from.
const APPLE_EPOCH: f64 = 978_307_200.0;
/// A line longer than this isn't the watcher's.
const LONGEST: usize = 16 * 1024;

/// The browsers Settings → Apps can switch on, by bundle id, with their id there. Each also
/// takes its editions and web apps (`com.google.Chrome.beta`, `com.google.Chrome.app.…`).
const BROWSERS: [(&str, &str); 5] = [
    ("com.google.chrome", "chrome"),
    ("com.brave.browser", "brave"),
    ("com.microsoft.edgemac", "edge"),
    ("company.thebrowser.browser", "arc"),
    ("com.apple.safari", "safari"),
];

/// MediaRemote's commands, as it numbers them.
const PLAY: u32 = 0;
const PAUSE: u32 = 1;
const NEXT: u32 = 4;
const PREVIOUS: u32 = 5;
const SEEK: u32 = 24;

/// What the watcher said.
#[derive(Debug, Clone, PartialEq)]
pub enum Heard {
    /// This Mac doesn't say what's playing; there's no point asking again.
    Unavailable,
    Read(Read),
}

/// One line from the watcher, as `WATCH` writes it.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Line {
    unavailable: bool,
    /// The app that plays, and the app it belongs to: Safari plays through WebKit.
    bundle: String,
    parent: String,
    title: String,
    artist: String,
    album: String,
    /// Seconds.
    duration: Option<f64>,
    elapsed: Option<f64>,
    /// Seconds since 2001 when `elapsed` was right.
    stamp: Option<f64>,
    playing: bool,
    /// What it answers to now.
    commands: Vec<u32>,
}

/// The Settings → Apps switch for the browser `bundle` belongs to, its editions and web apps
/// included.
pub fn switch(bundle: &str) -> Option<&'static str> {
    let bundle = bundle.to_ascii_lowercase();
    BROWSERS.iter().find_map(|(family, id)| match bundle.strip_prefix(family) {
        Some(rest) if rest.is_empty() || rest.starts_with('.') => Some(*id),
        _ => None,
    })
}

pub fn is_browser(bundle: &str) -> bool {
    switch(bundle).is_some()
}

/// Reads one line. `None` for one that isn't the watcher's. Something other than a browser
/// playing counts as no browser playing: Music, Spotify and TV are read on their own.
pub fn parse(line: &str, now: f64) -> Option<Heard> {
    if line.len() > LONGEST {
        return None;
    }
    let line: Line = serde_json::from_str(line.trim()).ok()?;
    if line.unavailable {
        return Some(Heard::Unavailable);
    }
    let app = [&line.bundle, &line.parent].into_iter().find(|bundle| is_browser(bundle));
    let Some(app) = app else { return Some(Heard::Read(Read::Closed)) };
    let text = |value: &str| Some(value.split_whitespace().collect::<Vec<_>>().join(" ")).filter(|value| !value.is_empty());
    let Some(title) = text(&line.title) else { return Some(Heard::Read(Read::Stopped)) };
    let artist = text(&line.artist);
    let seconds = |value: Option<f64>| value.filter(|value| value.is_finite() && *value >= 0.0);
    let at = seconds(line.stamp).map(|stamp| stamp + APPLE_EPOCH).filter(|at| *at <= now + 60.0).unwrap_or(now);
    let family = switch(app).unwrap_or_default();
    Some(Heard::Read(Read::Track(Reading {
        player: Player::Browser,
        playing: line.playing,
        track: format!("{family}\u{1f}{title}\u{1f}{}", artist.clone().unwrap_or_default()),
        title,
        artist,
        album: text(&line.album),
        duration: seconds(line.duration).filter(|duration| *duration > 0.0),
        position: seconds(line.elapsed),
        shuffle: None,
        repeat: None,
        volume: None,
        artwork_url: None,
        at,
        app: Some(app.clone()),
        previous: Some(line.commands.contains(&PREVIOUS)),
        next: Some(line.commands.contains(&NEXT)),
    })))
}

/// The running watcher. Dropping it stops it.
pub struct Watcher {
    _child: Child,
    lines: Lines<BufReader<ChildStdout>>,
}

impl Watcher {
    pub fn start() -> Result<Watcher, String> {
        let mut command = Command::new("/usr/bin/osascript");
        command
            .args(["-l", "JavaScript", "-e", WATCH])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let mut child = command.spawn().map_err(|problem| format!("The watcher didn't start: {problem}"))?;
        let stdout = child.stdout.take().ok_or("The watcher has no output.")?;
        Ok(Watcher { _child: child, lines: BufReader::new(stdout).lines() })
    }

    /// The next thing it says, skipping anything it couldn't have. `None` once it stopped.
    /// Safe to drop part way, in a `select!`.
    pub async fn next(&mut self) -> Option<Heard> {
        loop {
            let line = self.lines.next_line().await.ok()??;
            if let Some(heard) = parse(&line, unix_now()) {
                return Some(heard);
            }
        }
    }
}

/// Play, pause, next, previous, or a seek to `to` seconds, for the browser `app` while it's
/// what macOS says plays.
pub async fn control(action: &str, to: Option<f64>, app: &str) -> Result<(), String> {
    let command = match action {
        "play" => PLAY,
        "pause" => PAUSE,
        "next" => NEXT,
        "previous" => PREVIOUS,
        "seek" => SEEK,
        _ => return Err(format!("A browser doesn't take “{action}”.")),
    };
    let position = match command {
        SEEK => to.filter(|to| to.is_finite() && *to >= 0.0).ok_or("No position came with it.")?,
        _ => 0.0,
    };
    if !is_browser(app) {
        return Err(format!("{app} isn't a browser."));
    }
    let app = app.to_string();
    let sent = tokio::task::spawn_blocking(move || super::native::command(command, position, &app))
        .await
        .map_err(|_| "The control didn't go out.")?;
    match sent {
        super::native::Sent::Taken => Ok(()),
        super::native::Sent::Elsewhere => Err("Something else plays now.".into()),
        super::native::Sent::Refused => Err("The browser didn't take it.".into()),
    }
}

/// Writes one JSON line each time what plays changes, and once at the start. macOS says when
/// it changes; a read is made once that settles, since a page often names its next video a
/// moment before the channel, and every few seconds anyway. Stops when Buntini does: its
/// output closes, or the process that started it is gone.
const WATCH: &str = r#"
const FRAMEWORK = '/System/Library/PrivateFrameworks/MediaRemote.framework';
const SETTLE = 300;
const CHECK = 5000;
const CHANGES = [
  'kMRMediaRemoteNowPlayingInfoDidChangeNotification',
  'kMRMediaRemoteNowPlayingApplicationIsPlayingDidChangeNotification',
  'kMRMediaRemoteNowPlayingApplicationPlaybackStateDidChangeNotification',
  'kMRMediaRemoteNowPlayingApplicationDidChangeNotification',
  'kMRMediaRemoteNowPlayingPlayerDidChange',
  'kMRMediaRemoteNowPlayingApplicationClientStateDidChange',
  'kMRMediaRemoteSupportedCommandsDidChangeNotification',
];
function run() {
  ObjC.import('Foundation');
  const out = $.NSFileHandle.fileHandleWithStandardOutput;
  const say = (row) => out.writeData($(JSON.stringify(row) + '\n').dataUsingEncoding($.NSUTF8StringEncoding));
  const request = $.NSBundle.bundleWithPath(FRAMEWORK).load ? $.NSClassFromString('MRNowPlayingRequest') : $();
  if (request.isNil()) { say({ unavailable: true }); return; }
  ObjC.bindFunction('getppid', ['int', []]);
  ObjC.bindFunction('MRMediaRemoteRegisterForNowPlayingNotifications', ['void', ['id']]);
  const nil = (x) => x === undefined || x === null || (typeof x === 'object' && typeof x.isNil === 'function' && x.isNil());
  const text = (x) => { const v = nil(x) ? null : ObjC.unwrap(x); return typeof v === 'string' ? v : ''; };
  const number = (x) => { const v = nil(x) ? null : ObjC.unwrap(x); return typeof v === 'number' && isFinite(v) ? v : null; };
  const read = () => {
    const item = request.localNowPlayingItem;
    const m = nil(item) ? null : item.metadata;
    if (nil(m)) return {};
    const path = request.localNowPlayingPlayerPath;
    const client = nil(path) ? null : path.client;
    const commands = [];
    const supported = request.localSupportedCommands;
    if (!nil(supported)) {
      for (let i = 0; i < Number(supported.count); i++) {
        const info = supported.objectAtIndex(i);
        if (info.enabled === true) commands.push(Number(info.command));
      }
    }
    return {
      bundle: nil(client) ? '' : text(client.bundleIdentifier),
      parent: nil(client) ? '' : text(client.parentApplicationBundleIdentifier),
      title: text(m.title),
      artist: text(m.trackArtistName),
      album: text(m.albumName),
      duration: number(m.duration),
      elapsed: number(m.elapsedTime),
      stamp: number(m.elapsedTimeTimestamp),
      playing: request.localIsPlaying === true,
      commands,
    };
  };
  let due = 0;
  ObjC.registerSubclass({
    name: 'BuntiniNowPlayingWatch',
    methods: {
      'changed:': { types: ['void', ['id']], implementation: () => { due = Date.now() + SETTLE; } },
      'tick:': { types: ['void', ['id']], implementation: () => {} },
    },
  });
  const watch = $.BuntiniNowPlayingWatch.alloc.init;
  const center = $.NSNotificationCenter.defaultCenter;
  for (const name of CHANGES) center.addObserverSelectorNameObject(watch, 'changed:', $(name), $());
  $.MRMediaRemoteRegisterForNowPlayingNotifications($.NSOperationQueue.mainQueue.underlyingQueue);
  $.NSTimer.scheduledTimerWithTimeIntervalTargetSelectorUserInfoRepeats(CHECK / 1000, watch, 'tick:', $(), true);
  const parent = $.getppid();
  const loop = $.NSRunLoop.currentRunLoop;
  let checked = 0;
  let last = '';
  while ($.getppid() === parent) {
    const now = Date.now();
    if ((due > 0 && now >= due) || now - checked >= CHECK) {
      due = 0;
      checked = now;
      let line = '';
      try { line = JSON.stringify(read()); } catch (e) { line = '{}'; }
      if (line !== last) {
        out.writeData($(line + '\n').dataUsingEncoding($.NSUTF8StringEncoding));
        last = line;
      }
      continue;
    }
    const until = due > 0 ? due : checked + CHECK;
    loop.runModeBeforeDate($('kCFRunLoopDefaultMode'), $.NSDate.dateWithTimeIntervalSince1970(until / 1000));
  }
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn track(heard: Option<Heard>) -> Reading {
        match heard {
            Some(Heard::Read(Read::Track(reading))) => reading,
            other => panic!("{other:?}"),
        }
    }

    /// A YouTube video in Chrome, as the watcher wrote it on this Mac.
    const YOUTUBE: &str = r#"{"bundle":"com.google.Chrome","parent":"","title":"Big Buck Bunny 60fps 4K - Official Blender Foundation Short Film","artist":"Blender","album":"","duration":634.601,"elapsed":100.5,"stamp":816932189.106883,"playing":true,"commands":[24,3,2,1,0]}"#;

    #[test]
    fn a_youtube_video_reads_its_title_channel_and_place() {
        let reading = track(parse(YOUTUBE, 1_795_239_400.0));
        assert_eq!(reading.player, Player::Browser);
        assert_eq!(reading.title, "Big Buck Bunny 60fps 4K - Official Blender Foundation Short Film");
        assert_eq!(reading.artist.as_deref(), Some("Blender"));
        assert_eq!(reading.album, None);
        assert!(reading.playing);
        assert_eq!(reading.duration, Some(634.601));
        assert_eq!(reading.position, Some(100.5));
        // macOS counts from 2001; the island from 1970.
        assert!((reading.at - 1_795_239_389.106883).abs() < 1e-6);
        assert_eq!(reading.app.as_deref(), Some("com.google.Chrome"));
        assert_eq!((reading.previous, reading.next), (Some(false), Some(false)));
        assert_eq!((reading.shuffle, reading.repeat.as_deref(), reading.volume), (None, None, None));
        // The clock runs on from when macOS last had it right.
        assert_eq!(reading.position_now(1_795_239_399.106883).map(|at| (at * 10.0).round() / 10.0), Some(110.5));
    }

    #[test]
    fn a_pause_stops_the_clock_and_a_playlist_can_skip() {
        let paused = YOUTUBE.replace(r#""playing":true"#, r#""playing":false"#).replace("[24,3,2,1,0]", "[24,5,4,1,0]");
        let reading = track(parse(&paused, 1_795_239_400.0));
        assert!(!reading.playing);
        assert_eq!(reading.position_now(1_795_239_999.0), Some(100.5));
        assert_eq!((reading.previous, reading.next), (Some(true), Some(true)));
    }

    #[test]
    fn the_same_video_is_the_same_track_and_another_is_not() {
        let now = 1_795_239_400.0;
        let first = track(parse(YOUTUBE, now));
        let later = track(parse(&YOUTUBE.replace("100.5", "130.0"), now));
        assert_eq!(first.track, later.track);
        let next = track(parse(&YOUTUBE.replace("Big Buck Bunny", "Sintel"), now));
        assert_ne!(first.track, next.track);
    }

    #[test]
    fn other_apps_playing_count_as_no_browser_playing() {
        for bundle in ["com.apple.Music", "com.spotify.client", "com.apple.podcasts", "com.google.chromecast", "org.mozilla.firefox", ""] {
            let line = YOUTUBE.replace("com.google.Chrome", bundle);
            assert_eq!(parse(&line, 0.0), Some(Heard::Read(Read::Closed)), "{bundle}");
        }
        assert_eq!(parse("{}", 0.0), Some(Heard::Read(Read::Closed)));
    }

    #[test]
    fn editions_web_apps_and_safari_through_webkit_follow_their_browser_s_switch() {
        for (bundle, id) in [
            ("com.google.Chrome", "chrome"),
            ("com.google.Chrome.beta", "chrome"),
            ("com.google.Chrome.app.agimnkijcaahngcdmfeangaknmldooml", "chrome"),
            ("company.thebrowser.Browser", "arc"),
            ("com.microsoft.edgemac.Dev", "edge"),
            ("com.brave.Browser", "brave"),
            ("com.apple.Safari", "safari"),
        ] {
            assert_eq!(switch(bundle), Some(id), "{bundle}");
        }
        // No switch, so never shown.
        for bundle in ["org.mozilla.firefox", "company.thebrowser.dia", "com.apple.SafariTechnologyPreview"] {
            assert_eq!(switch(bundle), None, "{bundle}");
        }
        let safari = YOUTUBE.replace(r#""bundle":"com.google.Chrome","parent":"""#, r#""bundle":"com.apple.WebKit.GPU","parent":"com.apple.Safari""#);
        assert_eq!(track(parse(&safari, 0.0)).app.as_deref(), Some("com.apple.Safari"));
        assert!(!is_browser("com.apple.Safaria"));
        assert!(!is_browser("com.apple.Music"));
    }

    #[test]
    fn a_live_stream_has_no_length_and_a_tab_without_a_title_shows_nothing() {
        let live = YOUTUBE.replace(r#""duration":634.601"#, r#""duration":null"#);
        assert_eq!(track(parse(&live, 1_795_239_400.0)).duration, None);
        let untitled = YOUTUBE.replace("Big Buck Bunny 60fps 4K - Official Blender Foundation Short Film", "  ");
        assert_eq!(parse(&untitled, 0.0), Some(Heard::Read(Read::Stopped)));
    }

    #[test]
    fn titles_are_kept_on_one_line_and_odd_numbers_are_dropped() {
        let line = YOUTUBE.replace("Big Buck Bunny", "Big\\nBuck\\tBunny").replace("634.601", "-1").replace("100.5", "-3");
        let reading = track(parse(&line, 1_795_239_400.0));
        assert!(reading.title.starts_with("Big Buck Bunny 60fps"));
        assert_eq!((reading.duration, reading.position), (None, None));
    }

    #[test]
    fn a_clock_from_the_future_is_not_believed() {
        let line = YOUTUBE.replace("816932189.106883", "999999999.0");
        assert_eq!(track(parse(&line, 1_795_239_400.0)).at, 1_795_239_400.0);
    }

    #[test]
    fn the_watcher_saying_it_cannot_and_garbage_are_told_apart() {
        assert_eq!(parse(r#"{"unavailable":true}"#, 0.0), Some(Heard::Unavailable));
        assert_eq!(parse("undefined", 0.0), None);
        assert_eq!(parse("", 0.0), None);
        assert_eq!(parse(&format!(r#"{{"title":"{}"}}"#, "a".repeat(LONGEST)), 0.0), None);
    }

    /// None of these reach macOS: each is turned away first.
    #[test]
    fn only_known_controls_go_to_a_browser() {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        assert!(runtime.block_on(control("shuffle", None, "com.google.Chrome")).is_err());
        assert!(runtime.block_on(control("seek", None, "com.google.Chrome")).is_err());
        assert!(runtime.block_on(control("seek", Some(f64::NAN), "com.google.Chrome")).is_err());
        assert!(runtime.block_on(control("pause", None, "com.apple.Music")).is_err());
    }

    #[test]
    fn the_watcher_never_tells_an_application_anything() {
        assert!(!WATCH.contains("Application("), "no Apple Events, so no Automation prompt");
        assert!(!WATCH.contains("tell application"));
    }
}
