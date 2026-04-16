use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use chrono::{Days, Local, NaiveDate};
use clap::Parser;
use serde::{Deserialize, Serialize};

// ── CLI ──────────────────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(name = "nba", about = "Display NBA scores and upcoming games")]
struct Cli {
    /// Team tricode (e.g. MIA, LAL) to show next 6 games for that team
    #[arg(value_name = "TEAM")]
    team: Option<String>,

    /// Show games for a specific date (YYYY-MM-DD). Defaults to today/next game day.
    #[arg(short, long)]
    date: Option<String>,

    /// Show today's live scores even if no games are active
    #[arg(short, long)]
    live: bool,
}

// ── NBA API types ────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct ScoreboardResponse {
    scoreboard: Scoreboard,
}

#[derive(Debug, Deserialize)]
struct Scoreboard {
    #[serde(rename = "gameDate")]
    game_date: String,
    games: Vec<LiveGame>,
}

#[derive(Debug, Deserialize)]
struct LiveGame {
    #[serde(rename = "gameStatus")]
    game_status: i32,
    #[serde(rename = "gameStatusText")]
    game_status_text: String,
    #[serde(rename = "homeTeam")]
    home_team: LiveTeam,
    #[serde(rename = "awayTeam")]
    away_team: LiveTeam,
}

#[derive(Debug, Deserialize)]
struct LiveTeam {
    #[serde(rename = "teamTricode")]
    team_tricode: String,
    #[serde(rename = "teamName")]
    team_name: String,
    #[serde(rename = "teamCity")]
    team_city: String,
    score: i32,
    wins: i32,
    losses: i32,
}

// Schedule endpoint types — nullable string fields
#[derive(Debug, Deserialize)]
struct ScheduleResponse {
    #[serde(rename = "leagueSchedule")]
    league_schedule: LeagueSchedule,
}

#[derive(Debug, Deserialize)]
struct LeagueSchedule {
    #[serde(rename = "gameDates")]
    game_dates: Vec<ScheduleGameDate>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct ScheduleGameDate {
    #[serde(rename = "gameDate")]
    game_date: String,
    games: Vec<ScheduleGame>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct ScheduleGame {
    #[serde(rename = "gameStatus")]
    game_status: i32,
    #[serde(rename = "gameStatusText")]
    game_status_text: String,
    #[serde(rename = "homeTeam")]
    home_team: ScheduleTeam,
    #[serde(rename = "awayTeam")]
    away_team: ScheduleTeam,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct ScheduleTeam {
    #[serde(rename = "teamTricode")]
    team_tricode: Option<String>,
    #[serde(rename = "teamName")]
    team_name: Option<String>,
    #[serde(rename = "teamCity")]
    team_city: Option<String>,
    wins: i32,
    losses: i32,
    score: i32,
}

// ── Unified game type ────────────────────────────────────────────────────────

struct Game {
    game_status: i32,
    game_status_text: String,
    home_tricode: String,
    #[allow(dead_code)]
    home_city: String,
    #[allow(dead_code)]
    home_name: String,
    home_score: i32,
    home_wins: i32,
    home_losses: i32,
    away_tricode: String,
    #[allow(dead_code)]
    away_city: String,
    #[allow(dead_code)]
    away_name: String,
    away_score: i32,
    away_wins: i32,
    away_losses: i32,
}

impl Game {
    fn from_live(g: &LiveGame) -> Self {
        Self {
            game_status: g.game_status,
            game_status_text: g.game_status_text.trim().to_string(),
            home_tricode: g.home_team.team_tricode.clone(),
            home_city: g.home_team.team_city.clone(),
            home_name: g.home_team.team_name.clone(),
            home_score: g.home_team.score,
            home_wins: g.home_team.wins,
            home_losses: g.home_team.losses,
            away_tricode: g.away_team.team_tricode.clone(),
            away_city: g.away_team.team_city.clone(),
            away_name: g.away_team.team_name.clone(),
            away_score: g.away_team.score,
            away_wins: g.away_team.wins,
            away_losses: g.away_team.losses,
        }
    }

    fn from_schedule(g: &ScheduleGame) -> Self {
        Self {
            game_status: g.game_status,
            game_status_text: g.game_status_text.trim().to_string(),
            home_tricode: g.home_team.team_tricode.clone().unwrap_or_default(),
            home_city: g.home_team.team_city.clone().unwrap_or_default(),
            home_name: g.home_team.team_name.clone().unwrap_or_default(),
            home_score: g.home_team.score,
            home_wins: g.home_team.wins,
            home_losses: g.home_team.losses,
            away_tricode: g.away_team.team_tricode.clone().unwrap_or_default(),
            away_city: g.away_team.team_city.clone().unwrap_or_default(),
            away_name: g.away_team.team_name.clone().unwrap_or_default(),
            away_score: g.away_team.score,
            away_wins: g.away_team.wins,
            away_losses: g.away_team.losses,
        }
    }
}

// ── Fetch data ───────────────────────────────────────────────────────────────

fn build_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())
}

fn cache_path() -> Option<PathBuf> {
    dirs::cache_dir().map(|d| d.join("nba-rs").join("schedule.json"))
}

fn read_cache(today: NaiveDate) -> Option<Vec<ScheduleGameDate>> {
    let path = cache_path()?;
    let meta = fs::metadata(&path).ok()?;
    let age = SystemTime::now().duration_since(meta.modified().ok()?).ok()?;
    if age > Duration::from_secs(7 * 24 * 60 * 60) {
        return None;
    }
    let body = fs::read_to_string(&path).ok()?;
    let dates: Vec<ScheduleGameDate> = serde_json::from_str(&body).ok()?;
    // Ensure cache covers today
    let covers_today = dates.iter().any(|gd| {
        parse_schedule_date(&gd.game_date).is_some_and(|d| d >= today)
    });
    if covers_today { Some(dates) } else { None }
}

fn write_cache(dates: &[ScheduleGameDate]) {
    if let Some(path) = cache_path() {
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string(dates) {
            let _ = fs::write(&path, json);
        }
    }
}

fn filter_next_week(dates: &[ScheduleGameDate], today: NaiveDate) -> Vec<ScheduleGameDate> {
    let end = today + Days::new(7);
    dates.iter()
        .filter(|gd| {
            parse_schedule_date(&gd.game_date).is_some_and(|d| d >= today && d <= end)
        })
        .cloned()
        .collect()
}

async fn fetch_schedule() -> Result<Vec<ScheduleGameDate>, String> {
    let today = Local::now().date_naive();

    if let Some(dates) = read_cache(today) {
        return Ok(dates);
    }

    let client = build_client()?;
    let resp = client
        .get("https://cdn.nba.com/static/json/staticData/scheduleLeagueV2.json")
        .header("User-Agent", "nba-rs/0.1")
        .send()
        .await
        .map_err(|e| format!("Network error: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }

    let body = resp.text().await.map_err(|e| e.to_string())?;
    let data: ScheduleResponse =
        serde_json::from_str(&body).map_err(|e| format!("Parse error: {e}"))?;

    let week = filter_next_week(&data.league_schedule.game_dates, today);
    write_cache(&week);

    Ok(data.league_schedule.game_dates)
}

async fn fetch_live_scores() -> Result<(String, Vec<Game>), String> {
    let client = build_client()?;
    let resp = client
        .get("https://cdn.nba.com/static/json/liveData/scoreboard/todaysScoreboard_00.json")
        .header("User-Agent", "nba-rs/0.1")
        .send()
        .await
        .map_err(|e| format!("Network error: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }

    let body = resp.text().await.map_err(|e| e.to_string())?;
    let data: ScoreboardResponse =
        serde_json::from_str(&body).map_err(|e| format!("Parse error: {e}"))?;

    let games = data.scoreboard.games.iter().map(Game::from_live).collect();
    Ok((data.scoreboard.game_date, games))
}

fn parse_schedule_date(s: &str) -> Option<NaiveDate> {
    let date_part = s.split(' ').next()?;
    NaiveDate::parse_from_str(date_part, "%m/%d/%Y").ok()
}

fn games_for_date(schedule: &[ScheduleGameDate], date: NaiveDate) -> Vec<Game> {
    let date_str = date.format("%m/%d/%Y").to_string();
    schedule
        .iter()
        .find(|gd| gd.game_date.starts_with(&date_str))
        .map(|gd| gd.games.iter().map(Game::from_schedule).collect())
        .unwrap_or_default()
}

fn upcoming_team_games(schedule: &[ScheduleGameDate], team: &str, count: usize) -> Vec<(NaiveDate, Game)> {
    let today = Local::now().date_naive();
    let mut result = Vec::new();
    for gd in schedule {
        if let Some(d) = parse_schedule_date(&gd.game_date) {
            if d >= today {
                for sg in &gd.games {
                    let away = sg.away_team.team_tricode.as_deref().unwrap_or("");
                    let home = sg.home_team.team_tricode.as_deref().unwrap_or("");
                    if away.eq_ignore_ascii_case(team) || home.eq_ignore_ascii_case(team) {
                        result.push((d, Game::from_schedule(sg)));
                        if result.len() >= count {
                            return result;
                        }
                    }
                }
            }
        }
    }
    result
}

fn next_game_date(schedule: &[ScheduleGameDate], after: NaiveDate) -> Option<NaiveDate> {
    let start = after.checked_add_days(Days::new(1))?;
    for gd in schedule {
        if let Some(d) = parse_schedule_date(&gd.game_date) {
            if d >= start && !gd.games.is_empty() {
                return Some(d);
            }
        }
    }
    None
}

// ── Display ──────────────────────────────────────────────────────────────────

const W: usize = 24; // inner width of each card

fn hline(left: &str, fill: &str, right: &str) -> String {
    format!("{left}{}{right}", fill.repeat(W))
}

fn print_games(date_label: &str, games: &[Game]) {
    println!();
    println!("  \u{2500}\u{2500} NBA Scores \u{2014} {date_label} \u{2500}\u{2500}");

    if games.is_empty() {
        println!();
        println!("  No games scheduled.");
        println!();
        return;
    }

    // Lay out games in a 2-column grid
    let pairs: Vec<_> = games.chunks(2).collect();

    for chunk in &pairs {
        let top = hline("\u{256d}", "\u{2500}", "\u{256e}");
        match chunk.len() {
            2 => println!("  {top}  {top}"),
            _ => println!("  {top}"),
        }

        let cards: Vec<Vec<String>> = chunk.iter().map(|g| render_card(g)).collect();
        let max_lines = cards.iter().map(|c| c.len()).max().unwrap_or(0);

        let empty_inner = format!("\u{2502}{}\u{2502}", " ".repeat(W));
        for row in 0..max_lines {
            let left = cards[0].get(row).unwrap_or(&empty_inner);
            if cards.len() > 1 {
                let right = cards[1].get(row).unwrap_or(&empty_inner);
                println!("  {left}  {right}");
            } else {
                println!("  {left}");
            }
        }

        let bot = hline("\u{2570}", "\u{2500}", "\u{256f}");
        match chunk.len() {
            2 => println!("  {bot}  {bot}"),
            _ => println!("  {bot}"),
        }
    }
    println!();
}

fn render_card(game: &Game) -> Vec<String> {
    let mut lines = Vec::new();

    let away_tc = if game.away_tricode.is_empty() { "TBD" } else { &game.away_tricode };
    let home_tc = if game.home_tricode.is_empty() { "TBD" } else { &game.home_tricode };

    let is_live = game.game_status == 2;
    let is_final = game.game_status == 3;
    let has_score = game.game_status > 1;

    // Status line
    let status_text = if is_live {
        format!("\u{25cf} {}", game.game_status_text)
    } else if is_final {
        "Final".to_string()
    } else {
        game.game_status_text.clone()
    };
    lines.push(card_line(&format!(" {status_text}"), 1 + status_text.chars().count()));

    // Team lines
    for (tc, wins, losses, score, is_winner) in [
        (away_tc, game.away_wins, game.away_losses, game.away_score, has_score && game.away_score > game.home_score),
        (home_tc, game.home_wins, game.home_losses, game.home_score, has_score && game.home_score > game.away_score),
    ] {
        let rec = if wins > 0 || losses > 0 {
            format!(" ({wins}-{losses})")
        } else {
            String::new()
        };

        let score_str = if has_score {
            let marker = if is_winner { " *" } else { "  " };
            format!("{score:>3}{marker}")
        } else {
            String::new()
        };
        let score_vis = if has_score { 5 } else { 0 };

        let left_vis = 2 + 3 + rec.len();
        let right_vis = score_vis + 1;
        let gap = if W > left_vis + right_vis { W - left_vis - right_vis } else { 1 };

        let content = format!("  {tc}{rec}{}{score_str} ", " ".repeat(gap));
        lines.push(format!("\u{2502}{content}\u{2502}"));
    }

    lines
}

fn card_line(content: &str, visible_len: usize) -> String {
    let pad = if visible_len < W { W - visible_len } else { 0 };
    format!("\u{2502}{content}{}\u{2502}", " ".repeat(pad))
}

fn print_team_schedule(team: &str, games: &[(NaiveDate, Game)]) {
    let tc = team.to_ascii_uppercase();
    println!();
    println!("  \u{2500}\u{2500} {tc} Upcoming Games \u{2500}\u{2500}");
    println!();

    if games.is_empty() {
        println!("  No upcoming games found.");
        println!();
        return;
    }

    for (date, game) in games {
        let away_tc = if game.away_tricode.is_empty() { "TBD" } else { &game.away_tricode };
        let home_tc = if game.home_tricode.is_empty() { "TBD" } else { &game.home_tricode };

        let is_home = home_tc.eq_ignore_ascii_case(&tc);
        let opponent = if is_home { away_tc } else { home_tc };
        let loc = if is_home { "vs" } else { " @" };

        let day = date.format("%a %m/%d").to_string();
        let time = &game.game_status_text;

        println!("  {day}  {loc} {opponent:<3}  {time}");
    }
    println!();
}

// ── Main ─────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    let today = Local::now().date_naive();

    // "help" as a positional arg -> show help
    if cli.team.as_deref() == Some("help") {
        Cli::parse_from(["nba", "--help"]);
    }

    // Team schedule mode
    if let Some(ref team) = cli.team {
        let schedule = fetch_schedule().await?;
        let games = upcoming_team_games(&schedule, team, 6);
        print_team_schedule(team, &games);
        return Ok(());
    }

    // Determine target date
    let target_date = if let Some(ref ds) = cli.date {
        NaiveDate::parse_from_str(ds, "%Y-%m-%d")
            .map_err(|_| format!("Invalid date format: {ds} (expected YYYY-MM-DD)"))?
    } else {
        today
    };

    // If requesting today and --live, use live scoreboard
    if target_date == today && (cli.live || cli.date.is_none()) {
        match fetch_live_scores().await {
            Ok((date, games)) if !games.is_empty() => {
                let label = format!("{date} (live)");
                print_games(&label, &games);
                return Ok(());
            }
            _ => {}
        }
    }

    // Load schedule for any date
    let schedule = fetch_schedule().await?;
    let mut date = target_date;
    let mut games = games_for_date(&schedule, date);

    // If no explicit date and no games today, show next game day
    if cli.date.is_none() && games.is_empty() {
        if let Some(next) = next_game_date(&schedule, date) {
            date = next;
            games = games_for_date(&schedule, date);
        }
    }

    let label = if date == today {
        format!("{} (today)", date.format("%Y-%m-%d"))
    } else {
        date.format("%Y-%m-%d  %A").to_string()
    };

    print_games(&label, &games);

    Ok(())
}
