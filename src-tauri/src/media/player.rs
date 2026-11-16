//! Reading and changing Music, Spotify and TV through their own AppleScript. Every script
//! returns at once when its app isn't open, so nothing here ever launches a player. Play,
//! pause and skipping go through `apps::control_music`, the same path a chat request uses.
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Player {
    Spotify,
    Music,
    Tv,
}

impl Player {
    /// In the order a tie is settled: Spotify first, as `apps::player` does.
    pub const ALL: [Player; 3] = [Player::Spotify, Player::Music, Player::Tv];

    /// The id in Settings → Apps and on the wire.
    pub fn id(self) -> &'static str {
        match self {
            Player::Spotify => "spotify",
            Player::Music => "music",
            Player::Tv => "tv",
        }
    }

    pub fn from_id(id: &str) -> Option<Player> {
        Player::ALL.into_iter().find(|player| player.id() == id)
    }

    /// The name AppleScript and macOS's consent check know it by.
    pub fn app(self) -> &'static str {
        match self {
            Player::Spotify => "Spotify",
            Player::Music => "Music",
            Player::Tv => "TV",
        }
    }

    pub fn bundle(self) -> &'static str {
        match self {
            Player::Spotify => "com.spotify.client",
            Player::Music => "com.apple.Music",
            Player::Tv => "com.apple.TV",
        }
    }

    /// The pill's colour.
    pub fn tint(self) -> &'static str {
        match self {
            Player::Spotify => "#1ED760",
            Player::Music => "#FA2D48",
            Player::Tv => "#8EA2FF",
        }
    }

    pub fn index(self) -> usize {
        match self {
            Player::Spotify => 0,
            Player::Music => 1,
            Player::Tv => 2,
        }
    }

    /// Open right now, asked of macOS without sending the player anything.
    pub fn running(self) -> bool {
        crate::ax::pid(self.bundle()).is_some()
    }

    fn read_script(self) -> &'static str {
        match self {
            Player::Spotify => SPOTIFY_READ,
            Player::Music => MUSIC_READ,
            Player::Tv => TV_READ,
        }
    }

    fn set_script(self) -> &'static str {
        match self {
            Player::Spotify => SPOTIFY_SET,
            Player::Music => MUSIC_SET,
            Player::Tv => TV_SET,
        }
    }
}

/// What a player is doing.
#[derive(Debug, Clone, PartialEq)]
pub enum Read {
    /// Not open.
    Closed,
    /// Open with nothing loaded.
    Stopped,
    Track(Reading),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Reading {
    pub player: Player,
    pub playing: bool,
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    /// Seconds.
    pub duration: Option<f64>,
    pub position: Option<f64>,
    pub shuffle: Option<bool>,
    /// `off`, `one` or `all`.
    pub repeat: Option<String>,
    /// 0…1.
    pub volume: Option<f64>,
    /// Spotify's cover, on its image server.
    pub artwork_url: Option<String>,
    /// Stays the same for the same track.
    pub track: String,
    /// Unix seconds when `position` was read.
    pub at: f64,
}

impl Reading {
    /// Where the track is now, counting the time since it was read.
    pub fn position_now(&self, now: f64) -> Option<f64> {
        let position = self.position?;
        let moved = if self.playing { (now - self.at).max(0.0) } else { 0.0 };
        Some(match self.duration {
            Some(duration) if duration > 0.0 => (position + moved).min(duration),
            _ => position + moved,
        })
    }

    /// 0…1 through the track.
    pub fn progress(&self, now: f64) -> Option<f64> {
        let duration = self.duration.filter(|duration| *duration > 0.0)?;
        Some((self.position_now(now)? / duration).clamp(0.0, 1.0))
    }
}

pub fn unix_now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|since| since.as_secs_f64()).unwrap_or(0.0)
}

/// Reads one player. Call only when it's open and macOS already allows asking it.
pub async fn read(player: Player) -> Result<Read, String> {
    let output = run(player.read_script(), &[], 5).await?;
    Ok(parse(player, &output, unix_now()))
}

/// Seeks, shuffles, repeats or sets the player's own volume. `value` is milliseconds for a
/// seek, `true`/`false` for shuffle, `off`/`one`/`all` for repeat and 0–100 for volume.
pub async fn set(player: Player, what: &str, value: &str) -> Result<(), String> {
    match run(player.set_script(), &[what.to_string(), value.to_string()], 5).await?.trim() {
        "closed" => Err(format!("{} isn’t open.", player.app())),
        _ => Ok(()),
    }
}

/// Play, pause, next or previous, through the same path a chat request takes.
pub async fn control(player: Player, action: &str) -> Result<(), String> {
    crate::apps::control_music(player.id(), action, "", false).await.map(|_| ())
}

async fn run(source: &str, argv: &[String], seconds: u64) -> Result<String, String> {
    let mut command = crate::actions::osascript_command("AppleScript", &format!("{CLEAN}{source}"));
    command.args(argv);
    crate::actions::output(command, seconds).await
}

/// The read scripts' one line: state, title, artist, album, duration and position in
/// milliseconds, shuffle, repeat, volume 0–100, cover address, track id, then `end`, so
/// empty fields at the end survive.
pub fn parse(player: Player, output: &str, at: f64) -> Read {
    let line = output.trim_end_matches(['\n', '\r']);
    match line.trim() {
        "closed" => return Read::Closed,
        "stopped" | "" => return Read::Stopped,
        _ => {}
    }
    let parts: Vec<&str> = line.split('\t').collect();
    let [state, title, artist, album, duration, position, shuffle, repeat, volume, artwork, track, "end"] = parts.as_slice() else {
        return Read::Stopped;
    };
    let text = |value: &str| Some(value.trim().to_string()).filter(|value| !value.is_empty());
    let millis = |value: &str| value.trim().parse::<f64>().ok().filter(|ms| ms.is_finite() && *ms >= 0.0).map(|ms| ms / 1000.0);
    let title = text(title).or_else(|| text(album)).unwrap_or_default();
    if title.is_empty() {
        return Read::Stopped;
    }
    let artist = text(artist);
    let album = text(album);
    let track = text(track).unwrap_or_else(|| format!("{title}\u{1f}{}\u{1f}{}", artist.clone().unwrap_or_default(), album.clone().unwrap_or_default()));
    Read::Track(Reading {
        player,
        playing: matches!(state.trim(), "playing" | "fast forwarding" | "rewinding"),
        title,
        artist,
        album,
        duration: millis(duration).filter(|seconds| *seconds > 0.0),
        position: millis(position),
        shuffle: match shuffle.trim() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        },
        repeat: match (player, repeat.trim()) {
            (Player::Spotify, "true") => Some("all".into()),
            (Player::Spotify, "false") => Some("off".into()),
            (_, mode @ ("off" | "one" | "all")) => Some(mode.into()),
            _ => None,
        },
        volume: volume.trim().parse::<f64>().ok().filter(|volume| volume.is_finite()).map(|volume| (volume / 100.0).clamp(0.0, 1.0)),
        artwork_url: text(artwork).filter(|url| url.starts_with("https://")),
        track,
        at,
    })
}

/// Tabs and line breaks would break the line apart.
const CLEAN: &str = r#"on clean(t)
  if t is missing value then return ""
  set AppleScript's text item delimiters to {tab, return, linefeed}
  set parts to text items of (t as text)
  set AppleScript's text item delimiters to " "
  set t to parts as text
  set AppleScript's text item delimiters to ""
  return t
end clean
on ms(x)
  if x is missing value then return ""
  return ((x * 1000) div 1) as text
end ms
on asLine(fields)
  set AppleScript's text item delimiters to tab
  set out to (fields & {"end"}) as text
  set AppleScript's text item delimiters to ""
  return out
end asLine
"#;

const SPOTIFY_READ: &str = r#"on run argv
  if application "Spotify" is not running then return "closed"
  tell application "Spotify"
    with timeout of 3 seconds
      set ps to player state as text
      if ps is "stopped" then return "stopped"
      set t to current track
      -- Spotify gives a track's length in milliseconds.
      set d to ""
      try
        set d to (duration of t) as text
      end try
      set u to ""
      try
        set u to artwork url of t
      end try
      return my asLine({ps, my clean(name of t), my clean(artist of t), my clean(album of t), d, my ms(player position), (shuffling as text), (repeating as text), (sound volume as text), my clean(u), my clean(id of t)})
    end timeout
  end tell
end run
"#;

const MUSIC_READ: &str = r#"on run argv
  if application "Music" is not running then return "closed"
  tell application "Music"
    with timeout of 3 seconds
      set ps to player state as text
      if ps is "stopped" then return "stopped"
      set nm to ""
      set ar to ""
      set al to ""
      set d to missing value
      set pid to ""
      try
        set t to current track
        set nm to name of t
        set ar to artist of t
        set al to album of t
        set d to duration of t
        set pid to persistent ID of t
      end try
      -- A radio station names the song in its stream title.
      try
        set stt to current stream title
        if stt is not missing value and stt is not "" then set nm to stt
      end try
      return my asLine({ps, my clean(nm), my clean(ar), my clean(al), my ms(d), my ms(player position), (shuffle enabled as text), (song repeat as text), (sound volume as text), "", my clean(pid)})
    end timeout
  end tell
end run
"#;

// A show answers its name, the show and "Season 1, Episode 2" where Music answers a track,
// its artist and album, as `apps::TV` does.
const TV_READ: &str = r#"on run argv
  if application "TV" is not running then return "closed"
  tell application "TV"
    with timeout of 3 seconds
      set ps to player state as text
      if ps is "stopped" then return "stopped"
      set nm to ""
      set sh to ""
      set ep to ""
      set d to missing value
      set pid to ""
      try
        set t to current track
        set nm to name of t
        set d to duration of t
        set pid to persistent ID of t
        try
          set sh to show of t
          set sn to season number of t
          set en to episode number of t
          if sn > 0 and en > 0 then set ep to "Season " & sn & ", Episode " & en
        end try
      on error
        try
          set nm to current stream title
        end try
      end try
      return my asLine({ps, my clean(nm), my clean(sh), my clean(ep), my ms(d), my ms(player position), "", "", (sound volume as text), "", my clean(pid)})
    end timeout
  end tell
end run
"#;

// argv: what, value. Numbers arrive as whole numbers, so no decimal point depends on the
// Mac's region.
const SPOTIFY_SET: &str = r#"on run argv
  set what to item 1 of argv
  set v to item 2 of argv
  if application "Spotify" is not running then return "closed"
  tell application "Spotify"
    with timeout of 3 seconds
      if what is "seek" then
        set player position to (v as integer) / 1000
      else if what is "shuffle" then
        set shuffling to (v is "true")
      else if what is "repeat" then
        set repeating to (v is not "off")
      else if what is "volume" then
        set sound volume to (v as integer)
      end if
    end timeout
  end tell
  return "ok"
end run
"#;

const MUSIC_SET: &str = r#"on run argv
  set what to item 1 of argv
  set v to item 2 of argv
  if application "Music" is not running then return "closed"
  tell application "Music"
    with timeout of 3 seconds
      if what is "seek" then
        set player position to (v as integer) / 1000
      else if what is "shuffle" then
        set shuffle enabled to (v is "true")
      else if what is "repeat" then
        if v is "one" then
          set song repeat to one
        else if v is "all" then
          set song repeat to all
        else
          set song repeat to off
        end if
      else if what is "volume" then
        set sound volume to (v as integer)
      end if
    end timeout
  end tell
  return "ok"
end run
"#;

const TV_SET: &str = r#"on run argv
  set what to item 1 of argv
  set v to item 2 of argv
  if application "TV" is not running then return "closed"
  tell application "TV"
    with timeout of 3 seconds
      if what is "seek" then
        set player position to (v as integer) / 1000
      else if what is "volume" then
        set sound volume to (v as integer)
      end if
    end timeout
  end tell
  return "ok"
end run
"#;

/// Writes the cover of Music's or TV's current track to `path`, as the player stored it.
pub async fn save_artwork(player: Player, path: &std::path::Path) -> Result<bool, String> {
    let source = ARTWORK.replace("APP", player.app());
    let mut command = crate::actions::osascript_command("AppleScript", &source);
    command.arg(path.to_string_lossy().to_string());
    Ok(crate::actions::output(command, 8).await?.trim() == "ok")
}

// "APP" is Music or TV, never anything from outside. The file is written by the script, not
// by the player, so the player never learns where Buntini keeps it.
const ARTWORK: &str = r#"on run argv
  set p to item 1 of argv
  if application "APP" is not running then return "closed"
  tell application "APP"
    with timeout of 5 seconds
      try
        if (count of artworks of current track) is 0 then return "none"
        set d to raw data of artwork 1 of current track
      on error
        return "none"
      end try
    end timeout
  end tell
  set f to open for access (POSIX file p) with write permission
  try
    set eof f to 0
    write d to f
    close access f
  on error e
    close access f
    error e
  end try
  return "ok"
end run
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn track(read: Read) -> Reading {
        match read {
            Read::Track(reading) => reading,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn spotify_reads_in_milliseconds_and_seconds_alike() {
        let line = "playing\tInstant Crush\tDaft Punk\tRandom Access Memories\t337000\t61500\tfalse\ttrue\t60\thttps://i.scdn.co/image/ab67\tspotify:track:2cGx\tend\n";
        let reading = track(parse(Player::Spotify, line, 100.0));
        assert!(reading.playing);
        assert_eq!(reading.title, "Instant Crush");
        assert_eq!(reading.duration, Some(337.0));
        assert_eq!(reading.position, Some(61.5));
        assert_eq!(reading.shuffle, Some(false));
        assert_eq!(reading.repeat.as_deref(), Some("all"));
        assert_eq!(reading.volume, Some(0.6));
        assert_eq!(reading.artwork_url.as_deref(), Some("https://i.scdn.co/image/ab67"));
        assert_eq!(reading.track, "spotify:track:2cGx");
    }

    #[test]
    fn music_keeps_its_three_repeat_modes_and_empty_fields() {
        let line = "paused\tSo What\tMiles Davis\t\t562000\t0\ttrue\tone\t100\t\t\tend\n";
        let reading = track(parse(Player::Music, line, 0.0));
        assert!(!reading.playing);
        assert_eq!(reading.album, None);
        assert_eq!(reading.repeat.as_deref(), Some("one"));
        assert_eq!(reading.artwork_url, None);
        assert_eq!(reading.track, "So What\u{1f}Miles Davis\u{1f}");
    }

    #[test]
    fn closed_stopped_and_garbled_lines_show_nothing() {
        assert_eq!(parse(Player::Music, "closed\n", 0.0), Read::Closed);
        assert_eq!(parse(Player::Music, "stopped\n", 0.0), Read::Stopped);
        assert_eq!(parse(Player::Music, "playing\tonly\tthree\n", 0.0), Read::Stopped);
        assert_eq!(parse(Player::Spotify, "playing\t\t\t\t0\t0\tfalse\tfalse\t50\t\t\tend\n", 0.0), Read::Stopped);
    }

    #[test]
    fn a_cover_only_comes_from_https() {
        let line = "playing\tA\tB\tC\t1000\t0\tfalse\tfalse\t50\thttp://i.scdn.co/x\tid\tend\n";
        assert_eq!(track(parse(Player::Spotify, line, 0.0)).artwork_url, None);
    }

    #[test]
    fn the_position_runs_on_while_playing_and_stops_at_the_end() {
        let line = "playing\tA\tB\tC\t200000\t190000\tfalse\toff\t50\t\tid\tend\n";
        let reading = track(parse(Player::Music, line, 1_000.0));
        assert_eq!(reading.position_now(1_004.0), Some(194.0));
        assert_eq!(reading.position_now(1_100.0), Some(200.0));
        assert_eq!(reading.progress(1_000.0), Some(0.95));
        let paused = Reading { playing: false, ..reading };
        assert_eq!(paused.position_now(1_100.0), Some(190.0));
    }

    #[test]
    fn every_script_returns_before_telling_a_closed_player_anything() {
        for source in [SPOTIFY_READ, MUSIC_READ, TV_READ, SPOTIFY_SET, MUSIC_SET, TV_SET, ARTWORK] {
            let guard = source.find("is not running then return \"closed\"").expect("guarded");
            let tell = source.find("tell application").expect("tells");
            assert!(guard < tell);
        }
    }
}
