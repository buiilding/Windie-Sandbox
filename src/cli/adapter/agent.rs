//! Terminal adapter for explicit enrollment and foreground presence. No tool execution.

use crate::{
    agent::{self, Client, storage::Storage},
    device::*,
};
use anyhow::{Context, Result};

/// Run one typed agent action; secrets never appear in terminal output.
pub(super) async fn run(command: crate::cli::AgentCommand) -> Result<()> {
    let storage = Storage::open()?;
    let configured = std::env::var("WINDIE_AGENT_SERVER").unwrap_or_else(|_| PRODUCTION_API.into());
    let client = Client::new(&configured)?;
    let command_is_status = matches!(command, crate::cli::AgentCommand::Status);
    let _lock = if command_is_status {
        None
    } else {
        Some(storage.lock()?)
    };
    let existing = storage.load()?;
    if let Some(c) = &existing {
        anyhow::ensure!(
            c.server == client.server,
            "Stored credential belongs to another API origin; refusing to forward it"
        );
    }
    match command {
        crate::cli::AgentCommand::Connect => {
            let mut existing = existing;
            if let Some(c) = &existing {
                match client.own_device(c).await {
                    Ok(v) => {
                        let mut c = existing.take().unwrap();
                        c.device_id = Some(v.id);
                        storage.save(&c)?;
                        println!("Already registered: {}. Use windie agent run.", v.id);
                        return Ok(());
                    }
                    Err(e) if e.code == DeviceError::Unauthorized => {
                        if c.device_id.is_some() {
                            println!(
                                "Stored credential is invalid/revoked. Confirm the old registration was revoked in Computers first."
                            );
                            if !confirm("Archive this local credential and explicitly pair again?")
                                .await?
                            {
                                return Ok(());
                            }
                            storage.archive()?;
                            existing = None;
                        }
                    }
                    Err(e) => return Err(e.into()),
                }
            }
            let mut c = match existing {
                Some(c) => c,
                None => agent::fresh_credentials(client.server.clone())?,
            };
            storage.save(&c)?;
            tokio::select! {
                result=agent::enrollment::enroll(&client,&storage,&mut c,|view| async move {
                    println!("Browser-approved account ID: {}",view.account_label.as_deref().unwrap_or("unknown"));
                    println!("Compare this ID with the pairing page. This enables presence only, not tool execution.");
                    confirm("Link this computer to that account?").await
                },|progress| match progress {
                    agent::enrollment::Progress::Code(start)=>println!("Open {PAIRING_URL}\nEnter code: {}\nOnly approve the request you initiated on this computer.",start.code),
                    agent::enrollment::Progress::Registered(id)=>println!("Registered {id}. Run windie agent run."),
                })=>result,
                _=tokio::signal::ctrl_c()=>{
                    if let Some(start)=&c.started {
                        let _=client.request::<serde_json::Value>(reqwest::Method::POST,&format!("/v1/device-enrollments/{}/cancel",start.id),Some(&c.enrollment_secret),None::<&()>).await;
                    }
                    println!("Pairing interrupted. Local state retained for recovery.");Ok(())
                }
            }
        }
        crate::cli::AgentCommand::Status => {
            let Some(c) = existing else {
                println!("Not paired. Run windie agent connect.");
                return Ok(());
            };
            println!("Server: {}", c.server);
            match client.own_device(&c).await {
                Ok(v) => println!(
                    "Device: {}\nPresence: {}\nLast seen (Unix seconds): {:?}",
                    v.id,
                    if v.online { "online" } else { "offline" },
                    v.last_seen
                ),
                Err(e) if e.code == DeviceError::Unauthorized => println!(
                    "Credential inactive, invalid, or revoked. No automatic pairing attempted."
                ),
                Err(_) => println!("Presence unknown: API unreachable or unavailable."),
            };
            Ok(())
        }
        crate::cli::AgentCommand::Run => {
            let c = existing.context("Not paired. Run windie agent connect.")?;
            anyhow::ensure!(
                c.device_id.is_some(),
                "Pairing is incomplete. Run windie agent connect."
            );
            let mut lease = None;
            let result = tokio::select! {result=agent::presence(&client,&c,&mut lease,|s|println!("{s}"))=>result,_=tokio::signal::ctrl_c()=>Ok(())};
            if result.is_ok()
                && let Some(lease) = lease
            {
                let _ = client
                    .request::<Lease>(
                        reqwest::Method::POST,
                        "/v1/agent/disconnect",
                        Some(&c.device_secret),
                        Some(&LeaseRequest {
                            lease_id: lease.lease_id,
                        }),
                    )
                    .await;
            }
            result
        }
    }
}
async fn confirm(prompt: &str) -> Result<bool> {
    println!("{prompt} [yes/no]");
    // Do not leave a spawn_blocking stdin reader alive after Ctrl-C. Polling
    // readiness with a zero timeout keeps this future safely cancellable.
    #[cfg(unix)]
    {
        let mut answer = Vec::new();
        loop {
            let mut fd = libc::pollfd {
                fd: libc::STDIN_FILENO,
                events: libc::POLLIN,
                revents: 0,
            };
            let ready = unsafe { libc::poll(&mut fd, 1, 0) };
            if ready < 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            if ready > 0 {
                let mut byte = [0u8; 1];
                let read = unsafe { libc::read(libc::STDIN_FILENO, byte.as_mut_ptr().cast(), 1) };
                if read < 0 {
                    return Err(std::io::Error::last_os_error().into());
                }
                if read == 0 || byte[0] == b'\n' {
                    return Ok(answer == b"yes" || answer == b"yes\r");
                }
                answer.push(byte[0]);
                anyhow::ensure!(answer.len() <= 256, "Confirmation input too long");
            } else {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        }
    }
    #[cfg(not(unix))]
    anyhow::bail!("Agent enrollment currently requires Unix credential storage")
}
