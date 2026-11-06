//! A shell over plain SSH, for a machine with no tty7 on it: the Linux on this
//! tablet, say. It rides the same handle as a tty7 pane, so typing, closing
//! and the terminal's size go through `pane_input`, `pane_close` and
//! `pane_lease` unchanged, and the page keeps one terminal for both.
//!
//! The server's key is never taken on trust here: an unknown or changed one
//! comes back as `hostkey:<fingerprint>` for the person to judge, and the page
//! passes what it trusts as `fingerprint`. Passwords live in the page's memory
//! for the session and are not written down. Once one has worked, this
//! device's own key can be left on the server so it is not asked for again.

use std::path::Path;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use russh::keys::ssh_key::private::Ed25519Keypair;
use russh::keys::ssh_key::{HashAlg, LineEnding, PrivateKey, PublicKey};
use russh::keys::{PrivateKeyWithHashAlg, PublicKeyOrCertificate, decode_secret_key};
use russh::{ChannelMsg, client};
use serde::Deserialize;
use std::sync::atomic::Ordering;
use tauri::State;
use tauri::ipc::{Channel, InvokeResponseBody};
use tokio::sync::mpsc;
use tty7_mobile_proto::{PaneEvent, PaneRequest};

use crate::{AppState, CmdResult, Up};

const KEY_FILE: &str = "ssh_ed25519";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(12);

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Target {
    host: String,
    port: u16,
    user: String,
    password: Option<String>,
    /// The server's key as last trusted.
    fingerprint: Option<String>,
    /// Leave this device's key on the server once the password has worked.
    remember: bool,
    cols: u16,
    rows: u16,
}

struct Client {
    trusted: Option<String>,
    seen: Arc<StdMutex<Option<String>>>,
}

impl client::Handler for Client {
    type Error = russh::Error;

    async fn check_server_key(&mut self, key: &PublicKeyOrCertificate) -> Result<bool, Self::Error> {
        // A certificate is judged by the key it carries, like any other:
        // nothing here knows a certificate authority to check it against.
        let public: PublicKey = match key {
            PublicKeyOrCertificate::PublicKey { key, .. } => key.clone(),
            PublicKeyOrCertificate::Certificate(cert) => cert.public_key().clone().into(),
        };
        let fingerprint = public.fingerprint(HashAlg::Sha256).to_string();
        let same = self.trusted.as_deref() == Some(fingerprint.as_str());
        *self.seen.lock().unwrap_or_else(|e| e.into_inner()) = Some(fingerprint);
        Ok(same)
    }
}

/// This device's key, made the first time it is wanted.
fn device_key(dir: &Path) -> Result<PrivateKey, String> {
    let path = dir.join(KEY_FILE);
    if let Ok(text) = std::fs::read_to_string(&path) {
        return decode_secret_key(&text, None).map_err(|e| format!("couldn't read this device's key: {e}"));
    }
    let mut seed = [0u8; 32];
    std::io::Read::read_exact(
        &mut std::fs::File::open("/dev/urandom").map_err(|e| e.to_string())?,
        &mut seed,
    )
    .map_err(|e| e.to_string())?;
    let key = PrivateKey::from(Ed25519Keypair::from_seed(&seed));
    let text = key.to_openssh(LineEnding::LF).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    std::fs::write(&path, text.as_bytes()).map_err(|e| e.to_string())?;
    Ok(key)
}

fn public_line(key: &PrivateKey) -> Result<String, String> {
    let mut public = key.public_key().clone();
    public.set_comment("tty7-pad");
    public.to_openssh().map_err(|e| e.to_string())
}

/// This device's public key, to put in a server's `authorized_keys`.
#[tauri::command]
pub fn ssh_public_key(state: State<'_, Arc<AppState>>) -> CmdResult<String> {
    public_line(&device_key(&state.dir)?)
}

/// Runs one command and waits for it, for leaving the key behind. The line is
/// base64 and a comment of our own, so it needs no quoting beyond the quotes.
async fn add_key(session: &client::Handle<Client>, line: &str) -> Result<(), russh::Error> {
    let mut channel = session.channel_open_session().await?;
    let command = format!(
        "umask 077; mkdir -p ~/.ssh && touch ~/.ssh/authorized_keys && \
         (grep -qxF '{line}' ~/.ssh/authorized_keys || echo '{line}' >> ~/.ssh/authorized_keys)"
    );
    channel.exec(true, command).await?;
    while let Some(msg) = channel.wait().await {
        if matches!(msg, ChannelMsg::ExitStatus { .. } | ChannelMsg::Close) {
            break;
        }
    }
    Ok(())
}

fn event(out: &Channel<InvokeResponseBody>, event: &PaneEvent) -> bool {
    let json = serde_json::to_string(event).expect("events serialize");
    out.send(InvokeResponseBody::Json(json)).is_ok()
}

#[tauri::command]
pub async fn ssh_open(
    state: State<'_, Arc<AppState>>,
    target: Target,
    on_output: Channel<InvokeResponseBody>,
) -> CmdResult<u32> {
    let key = device_key(&state.dir)?;
    let seen = Arc::new(StdMutex::new(None));
    let handler = Client {
        trusted: target.fingerprint.clone(),
        seen: seen.clone(),
    };
    let config = Arc::new(client::Config {
        keepalive_interval: Some(Duration::from_secs(30)),
        ..Default::default()
    });
    let address = (target.host.as_str(), target.port);
    let place = format!("{}:{}", target.host, target.port);
    let connected = tokio::time::timeout(CONNECT_TIMEOUT, client::connect(config, address, handler)).await;
    let mut session = match connected {
        Err(_) => return Err(format!("{place} didn't answer")),
        Ok(Ok(session)) => session,
        Ok(Err(e)) => {
            let fingerprint = seen.lock().unwrap_or_else(|e| e.into_inner()).clone();
            return Err(match fingerprint {
                Some(fingerprint) if target.fingerprint.as_ref() != Some(&fingerprint) => {
                    format!("hostkey:{fingerprint}")
                }
                _ => format!("couldn't reach {place}: {e}"),
            });
        }
    };

    let line = public_line(&key)?;
    let signed = PrivateKeyWithHashAlg::new(Arc::new(key), None);
    let by_key = session
        .authenticate_publickey(&target.user, signed)
        .await
        .map_err(|e| e.to_string())?
        .success();
    if !by_key {
        let Some(password) = target.password.as_deref() else {
            return Err("auth:password".into());
        };
        let by_password = session
            .authenticate_password(&target.user, password)
            .await
            .map_err(|e| e.to_string())?
            .success();
        if !by_password {
            return Err("auth:wrong".into());
        }
        if target.remember {
            // Worth having, not worth failing over: the password still works.
            let _ = tokio::time::timeout(Duration::from_secs(5), add_key(&session, &line)).await;
        }
    }

    let mut channel = session.channel_open_session().await.map_err(|e| e.to_string())?;
    channel
        .request_pty(true, "xterm-256color", target.cols.into(), target.rows.into(), 0, 0, &[])
        .await
        .map_err(|e| e.to_string())?;
    channel.request_shell(true).await.map_err(|e| e.to_string())?;

    let handle = state.next_pane.fetch_add(1, Ordering::Relaxed);
    let (input_tx, mut input_rx) = mpsc::unbounded_channel::<Up>();
    state.panes.lock().await.insert(handle, input_tx);
    event(
        &on_output,
        &PaneEvent::Size {
            cols: target.cols,
            rows: target.rows,
        },
    );

    let app_state = state.inner().clone();
    tokio::spawn(async move {
        // Held for as long as the shell runs: dropping it disconnects.
        let session = session;
        let mut code = None;
        loop {
            tokio::select! {
                up = input_rx.recv() => match up {
                    // The page let go of the pane.
                    None => break,
                    Some(Up::Keys(bytes)) => {
                        if let Err(e) = channel.data(&bytes[..]).await {
                            event(&on_output, &PaneEvent::Error {
                                message: format!("typing didn't reach the machine: {e}"),
                            });
                            break;
                        }
                    }
                    Some(Up::Request(PaneRequest::TakeOver { size })) => {
                        let _ = channel.window_change(size.cols.into(), size.rows.into(), 0, 0).await;
                        event(&on_output, &PaneEvent::Lease { held: true, refused: None });
                        // The daemon says the pane's new size itself; here it
                        // is ours to say, or the terminal keeps its old grid.
                        event(&on_output, &PaneEvent::Size { cols: size.cols, rows: size.rows });
                    }
                    Some(Up::Request(_)) => {}
                },
                msg = channel.wait() => match msg {
                    Some(ChannelMsg::Data { data }) | Some(ChannelMsg::ExtendedData { data, .. }) => {
                        if on_output.send(InvokeResponseBody::Raw(data.to_vec())).is_err() {
                            break;
                        }
                    }
                    Some(ChannelMsg::ExitStatus { exit_status }) => code = Some(exit_status as i32),
                    Some(ChannelMsg::Eof) | Some(ChannelMsg::Close) | None => {
                        event(&on_output, &PaneEvent::Exited { code });
                        break;
                    }
                    Some(_) => {}
                },
            }
        }
        let _ = session
            .disconnect(russh::Disconnect::ByApplication, "", "en")
            .await;
        app_state.panes.lock().await.remove(&handle);
    });
    Ok(handle)
}
