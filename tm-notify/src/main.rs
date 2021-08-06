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

use tracing::{info, instrument, warn, Level};
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
#[derive(Debug)]
struct Opts {
    /// slack room to beep boop in
    webhook: Option<String>,
    /// bucket for state
    bucket: String,
}

#[derive(Debug, PartialEq, Eq, Deserialize)]
struct ViewGameResponse {
    finished: Option<i32>,
    active_faction: Option<String>,
    // TODO: Is this nullable? If there are no actions required or if the game
    // is over, is the value an empty array or missing?
    action_required: Option<Vec<ActionRequired>>,
}

// There are different types of actions required. For example, during faction
// selection, `faction` is missing but `player` is present. Might be nice to
// model this as an enum. For now, we use Optionals.
#[derive(Debug, PartialEq, Eq, Deserialize)]
struct ActionRequired {
    from_faction: Option<String>,
    r#type: Option<String>,
    faction: Option<String>,
    player: Option<String>,
}

/// The webhook is a secret and is injected at build time.
const WEBHOOK: Option<&'static str> = std::option_env!("WEBHOOK");
/// The list of games to monitor. New games require a new build!
const GAME_IDS: &[&'static str] = &["terramysticians20210803"];

async fn run() -> Result<()> {
    info!("running");
    // FIXME: Inject somehow.
    let opts = Opts {
        webhook: WEBHOOK.map(|url| url.into()),
        bucket: "cdkstack-tmnotifytmnotifyvara98b5e04-1i9nxaq6v6ckn".into(),
    };

    for id in GAME_IDS {
        process(&id, &opts).await?;
    }

    Ok(())
}

#[instrument]
async fn process(game_id: &str, opts: &Opts) -> Result<()> {
    let (gamedata, game) = load_game(game_id).await?;

    if !is_game_changed(game_id, &game, &opts).await? {
        return Ok(());
    }

    if let Some(message) = notification_message(&game)? {
        if let Some(url) = &opts.webhook {
            notify(game_id, message, url).await?;
        } else {
            info!("no webhook, not sending a notification");
        }
    }
    upload_gamefile(game_id, gamedata, &opts).await?;

    Ok(())
}

async fn load_game(game_id: &str) -> Result<(Bytes, ViewGameResponse)> {
    info!("requesting latest game information");
    let client = reqwest::Client::new();
    let params = [("game", game_id)];
    let resp = client
        .post("https://terra.snellman.net/app/view-game/")
        .form(&params)
        .header("Accept", "application/json")
        .send()
        .await?;
    let gamedata = resp.bytes().await?;
    let view = serde_json::from_slice(gamedata.as_ref())?;
    Ok((gamedata, view))
}

async fn is_game_changed(game_id: &str, game: &ViewGameResponse, opts: &Opts) -> Result<bool> {
    let config = Config::builder().build();
    let client = Client::from_conf(config);
    let resp = client
        .get_object()
        .bucket(&opts.bucket)
        .key(format!("{}/{}.json", "games", game_id))
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
        info!("game has not been updated");
        return Ok(false);
    } else {
        info!("game has been updated");
        return Ok(true);
    }
}

async fn upload_gamefile(game_id: &str, gamedata: Bytes, opts: &Opts) -> Result<()> {
    info!("saving gamefile");

    let config = Config::builder().build();
    let client = Client::from_conf(config);
    let _ = client
        .put_object()
        .bucket(&opts.bucket)
        .key(format!("{}/{}.json", "games", game_id))
        .body(ByteStream::from(gamedata))
        .send()
        .await?;

    Ok(())
}

async fn notify(game_id: &str, message: String, webhook: &str) -> Result<()> {
    let mut notification = HashMap::new();
    notification.insert("game_id", game_id);
    notification.insert("message", &message[..]);
    let client = reqwest::Client::new();
    let resp = client.post(webhook).json(&notification).send().await?;

    if !resp.status().is_success() {
        Err(anyhow!("webhook failed with {}", resp.status().as_u16()))
    } else {
        info!(%message, "notification sent");
        Ok(())
    }
}

fn notification_message(game: &ViewGameResponse) -> Result<Option<String>> {
    if let Some(1) = game.finished {
        return Ok(Some("gameover".into()));
    }

    Ok(if let Some(ref action_required) = game.action_required {
        // To minimize noise, we skip lingering actions (such as leech) if another player must take a full turn.
        let is_full_turn = action_required.iter().any(|it| match it.r#type {
            Some(ref t) => match &t[..] {
                "full" => true,
                _ => false,
            },
            _ => false,
        });

        let message = match is_full_turn {
            true => notify_full_turn(&game)?,
            false => notify_lingering(&action_required),
        };

        Some(message)
    } else {
        warn!("not sure how to build a notification");
        None
    })
}

fn notify_full_turn(game: &ViewGameResponse) -> Result<String> {
    match game.active_faction {
        Some(ref active_faction) => Ok(format!("{} should take their turn", active_faction)),
        None => Err(anyhow!("full turn required but no active faction??")),
    }
}

fn notify_lingering(actions_required: &[ActionRequired]) -> String {
    let mut notify = vec![];

    // https://github.com/jsnell/terra-mystica/blob/f8a4e19246177f09fa3c1a217bcb3d353f05d761/stc/game.js#L1774
    for it in actions_required {
        if let (Some(faction), Some(r#type)) = (&it.faction, &it.r#type) {
            let it = match &r#type[..] {
                "dwelling" => format!("{} should place a dwelling", faction),
                // "leech" => format!("{} may gain power", faction),
                // "transform" => format!("{} may transform", faction),
                "cult" => format!("{} may advance on a cult track", faction),
                "bonus" => format!("{} should pick a bonus tile", faction),
                _ => format!("{} may {}", faction, r#type),
            };

            notify.push(it);
        }

        if let (Some(player), Some(r#type)) = (&it.player, &it.r#type) {
            if r#type == "faction" {
                notify.push(format!("{} should pick a faction", player));
            } else {
                // pretty sure this is dead code.
                notify.push(format!("{} may {}", player, r#type));
            }
        }
    }

    notify.join("\n")
}

#[cfg(test)]
mod test {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn notify_faction_selection() -> Result<()> {
        // taken from terramysticians20210803
        let example = json!({
            "active_faction": null,
            "action_required":[
                {"player_index":"player2","player":"Johanvr","type":"faction"}
            ]
        });

        let game: ViewGameResponse = serde_json::from_value(example)?;
        let message = notification_message(&game)?;
        assert_eq!("Johanvr should pick a faction", message.as_ref().unwrap());

        Ok(())
    }

    #[tokio::test]
    async fn notify_full_turn() -> Result<()> {
        // taken from terramysticians20210714
        let example = json!({
            "active_faction": "witches",
            "action_required":[
                {"from_faction":"cultists","actual":2,"leech_id":82,"amount":2,"type":"leech","faction":"witches"},
                {"type":"full","faction":"witches"}
            ],
        });

        let game: ViewGameResponse = serde_json::from_value(example)?;
        let message = notification_message(&game)?;
        assert_eq!("witches should take their turn", message.as_ref().unwrap());

        Ok(())
    }

    #[tokio::test]
    async fn notify_finished() -> Result<()> {
        // taken from terramysticians20210714
        let example = json!({
            "finished": 1,
            "active_faction": "cultists",
            "action_required": [
                {
                    "type": "gameover"
                }
            ],
        });

        let game: ViewGameResponse = serde_json::from_value(example)?;
        let message = notification_message(&game)?;
        assert_eq!("gameover", message.as_ref().unwrap());

        Ok(())
    }
}
