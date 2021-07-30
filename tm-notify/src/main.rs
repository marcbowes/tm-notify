use std::collections::HashMap;
use std::result::Result as StdResult;

use anyhow::{anyhow, Result};
use aws_sdk_s3::error::GetObjectErrorKind;
use aws_sdk_s3::{ByteStream, Client, Config};
use bytes::Bytes;
use lambda_runtime::Error as LambdaError;
use lambda_runtime::{handler_fn, Context};
use serde::Deserialize;
use serde_json;

use tracing::{info, Level};
use tracing_subscriber::util::SubscriberInitExt;

#[tokio::main]
async fn main() -> StdResult<(), LambdaError> {
    let func = handler_fn(func);
    tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .finish()
        .try_init()?;
    lambda_runtime::run(func).await?;
    Ok(())
}

async fn func(_: serde_json::Value, _: Context) -> StdResult<serde_json::Value, LambdaError> {
    match run().await {
        Ok(()) => Ok(serde_json::Value::Null),
        Err(err) => Err(err.into()),
    }
}

/// Monitor a game and send updates to a slack room
struct Opts {
    /// game to monitor
    game: String,
    /// slack room to beep boop in
    webhook: Option<String>,
    /// bucket for state
    bucket: String,
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

/// The webhook is a secret and is injected at build time.
const WEBHOOK: Option<&'static str> = std::option_env!("WEBHOOK");

async fn run() -> Result<()> {
    info!("running");
    // FIXME: Inject somehow.
    let opts = Opts {
        game: "terramysticians20210714".into(),
        webhook: WEBHOOK.map(|url| url.into()),
        bucket: "cdkstack-tmnotifytmnotifyvara98b5e04-1i9nxaq6v6ckn".into(),
    };

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

    if !is_game_changed(&game, &opts).await? {
        return Ok(());
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

    if let Some(url) = &opts.webhook {
        notify(message, url).await?;
    } else {
        info!("no webhook, not sending a notification");
    }
    upload_gamefile(gamedata, &opts).await?;

    Ok(())
}

async fn is_game_changed(game: &ViewGameResponse, opts: &Opts) -> Result<bool> {
    let config = Config::builder().build();
    let client = Client::from_conf(config);
    let resp = client
        .get_object()
        .bucket(&opts.bucket)
        .key(format!("{}/{}.json", "games", opts.game))
        .send()
        .await;

    let body = match resp {
        Ok(output) => output.body.collect().await?.into_bytes(),
        Err(err) => match err {
            // If the key doesn't exist, assume this is a new game.
            aws_sdk_s3::SdkError::ServiceError { err, .. } => {
                if let GetObjectErrorKind::NoSuchKey(it) = err.kind {
                    info!(
                        %it,
                        "no such key, assuming this is a new game"
                    );
                    return Ok(true);
                } else {
                    return Err(err)?;
                }
            }
            _ => Err(err)?,
        },
    };

    // Note: I found comparing bytes to be inadequate, and so we work with
    // the deserialized version. I'm not sure if list ordering is stable.
    let cached_game: ViewGameResponse = serde_json::from_slice(&body[..])?;
    if &cached_game == game {
        info!(game = %opts.game, "game has not been updated");
        return Ok(false);
    } else {
        info!(game = %opts.game, "game has been updated");
        return Ok(true);
    }
}

async fn upload_gamefile(gamedata: Bytes, opts: &Opts) -> Result<()> {
    info!("saving gamefile");

    let config = Config::builder().build();
    let client = Client::from_conf(config);
    let _ = client
        .put_object()
        .bucket(&opts.bucket)
        .key(format!("{}/{}.json", "games", opts.game))
        .body(ByteStream::from(gamedata))
        .send()
        .await?;

    Ok(())
}

async fn notify(message: String, webhook: &str) -> Result<()> {
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
