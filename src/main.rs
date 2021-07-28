use std::{
    collections::HashMap,
    fs::{self, File},
    io::{Read, Write},
    path::Path,
};

use anyhow::{anyhow, Result};
use argh::FromArgs;
use serde::{Deserialize, Serialize};
use serde_json;

use tracing::info;

#[derive(FromArgs)]
/// Monitor a game and send updates to a slack room
struct Opts {
    #[argh(option)]
    /// game to monitor
    game: String,
    #[argh(option)]
    /// slack room to beep boop in
    webhook: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let opts = argh::from_env();
    tracing_subscriber::fmt::init();
    run(opts).await
}

#[derive(PartialEq, Eq, Deserialize)]
struct ViewGameResponse {
    active_faction: Option<String>,
    // TODO: Is this nullable? If there are no actions required or if the game
    // is over, is the value an empty array or missing?
    action_required: Option<Vec<ActionRequired>>,
}

#[derive(PartialEq, Eq, Deserialize)]
// TODO: I'm not sure what the possible fields and values are, so for now
// everything I know _might_ be required is optional!
struct ActionRequired {
    from_faction: Option<String>,
    r#type: Option<String>,
    faction: Option<String>,
}

async fn run(opts: Opts) -> Result<()> {
    info!("running");

    info!("requesting latest game information");
    let client = reqwest::Client::new();
    let params = [("game", &opts.game[..])];
    let resp = client
        .post("https://terra.snellman.net/app/view-game/")
        .form(&params)
        .header("Accept", "application/json")
        .send()
        .await?;
    let gamedata = resp.bytes().await?;
    let game: ViewGameResponse = serde_json::from_slice(gamedata.as_ref())?;

    let cache_dir = Path::new("/tmp").join("tm-notify");
    let cache_gamefile = cache_dir.join(format!("{}.json", &opts.game[..]));

    if cache_gamefile.exists() {
        info!(
            file = cache_gamefile.to_str().unwrap_or("?"),
            "loading previous gamefile"
        );
        let mut buf = vec![];
        let mut file = File::open(&cache_gamefile)?;
        file.read_to_end(&mut buf)?;

        // Note: I found comparing bytes to be inadequate, and so we work with
        // the deserialized version. I'm not sure if list ordering is stable.
        let cached_game: ViewGameResponse = serde_json::from_slice(&buf[..])?;
        if cached_game == game {
            info!(game = %opts.game, "game has not been updated");
            return Ok(());
        } else {
            info!(game = %opts.game, "game has been updated");
        }
    } else {
        info!("no previous gamefile");
    }

    let message = if let Some(ref action_required) = game.action_required {
        let is_full_turn = action_required.iter().any(|it| match it.r#type {
            Some(ref t) if &t[..] == "full" => true,
            _ => false,
        });

        match is_full_turn {
            true => notify_full_turn(&game)?,
            false => notify_lingering(&action_required),
        }
    } else {
        // I'm assuming no actions required means the game is over.
        return Ok(());
    };

    notify(message, opts.webhook).await?;

    info!(
        file = cache_gamefile.to_str().unwrap_or("?"),
        "saving gamefile"
    );
    fs::create_dir_all(cache_dir)?;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&cache_gamefile)?;
    file.write_all(gamedata.as_ref())?;

    Ok(())
}

async fn notify(message: String, webhook: String) -> Result<()> {
    let mut notification = HashMap::new();
    notification.insert("message", message);
    let client = reqwest::Client::new();
    let resp = client.post(webhook).json(&notification).send().await?;

    if !resp.status().is_success() {
        Err(anyhow!("webhook failed with {}", resp.status().as_u16()))
    } else {
        Ok(())
    }
}

fn notify_full_turn(game: &ViewGameResponse) -> Result<String> {
    match game.active_faction {
        Some(ref active_faction) => Ok(format!("{} should take their turn", active_faction)),
        None => Err(anyhow!("full turn required but no active faction??")),
    }
}

fn notify_lingering(actions_required: &[ActionRequired]) -> String {
    let mut notify = vec![];

    for it in actions_required {
        if let (Some(faction), Some(r#type)) = (&it.faction, &it.r#type) {
            notify.push(format!("{} may {}", faction, r#type));
        }
    }

    notify.join("\n")
}
