//! Recoverable enrollment workflow. Terminal consent and rendering are injected by the CLI.

use super::{
    Client,
    storage::{Credentials, Storage},
};
use crate::device::*;
use anyhow::{Context, Result, bail};

/// Public pairing details only; bearer secrets are never rendered.
pub(crate) enum Progress {
    Code(EnrollmentStarted),
    Registered(DeviceId),
}

/// The caller has already persisted both independent secrets and holds the OS
/// lock. This workflow can be dropped at any await without losing credentials.
pub(crate) async fn enroll<F, Fut>(
    client: &Client,
    storage: &Storage,
    c: &mut Credentials,
    mut confirm: F,
    mut report: impl FnMut(Progress),
) -> Result<()>
where
    F: FnMut(EnrollmentView) -> Fut,
    Fut: std::future::Future<Output = Result<bool>>,
{
    let deadline =
        tokio::time::Instant::now() + std::time::Duration::from_secs(ENROLLMENT_SECONDS as u64);
    let mut attempt = 0;
    if c.started.is_none() {
        loop {
            if tokio::time::Instant::now() >= deadline {
                bail!("Enrollment initiation timed out; saved state retained for retry.");
            }
            match client
                .request::<EnrollmentStarted>(
                    reqwest::Method::POST,
                    "/v1/device-enrollments",
                    None,
                    Some(&c.request),
                )
                .await
            {
                Ok(start) => {
                    c.started = Some(start);
                    storage.save(c)?;
                    break;
                }
                Err(e) if e.retryable() && tokio::time::Instant::now() < deadline => {
                    wait_retry(deadline, attempt, e.retry_after).await;
                    attempt += 1;
                }
                Err(e) if e.code == DeviceError::Expired => {
                    storage.archive()?;
                    bail!("Pending enrollment expired. Rerun connect to explicitly start again.");
                }
                Err(e) => return Err(e.into()),
            }
        }
    }
    let start = c.started.as_ref().unwrap().clone();
    // Never trust a remote response to redirect a user to an arbitrary pairing site.
    report(Progress::Code(start.clone()));
    loop {
        if tokio::time::Instant::now() >= deadline {
            bail!("Pairing wait expired; state retained. Rerun connect to inspect recovery.");
        }
        let v = match client
            .request::<EnrollmentView>(
                reqwest::Method::GET,
                &format!("/v1/device-enrollments/{}", start.id),
                Some(&c.enrollment_secret),
                None::<&()>,
            )
            .await
        {
            Ok(v) => v,
            Err(e) if e.retryable() => {
                wait_retry(deadline, attempt, e.retry_after.max(POLL_SECONDS)).await;
                attempt = attempt.saturating_add(1);
                continue;
            }
            Err(e) if e.code == DeviceError::Expired => {
                storage.archive()?;
                bail!("Pending enrollment expired. Rerun connect to explicitly start again.");
            }
            Err(e) => return Err(e.into()),
        };
        match v.state {
            EnrollmentState::Approved | EnrollmentState::Consumed => {
                if v.state != EnrollmentState::Consumed && !confirm(v.clone()).await? {
                    client
                        .request::<serde_json::Value>(
                            reqwest::Method::POST,
                            &format!("/v1/device-enrollments/{}/cancel", start.id),
                            Some(&c.enrollment_secret),
                            None::<&()>,
                        )
                        .await?;
                    storage.archive()?;
                    return Ok(());
                }
                let account_id = v.account_id.context("Missing approved account binding")?;
                loop {
                    if tokio::time::Instant::now() >= deadline {
                        bail!(
                            "Finalization timed out; rerun connect to recover using saved credentials."
                        );
                    }
                    match client
                        .request::<serde_json::Value>(
                            reqwest::Method::POST,
                            &format!("/v1/device-enrollments/{}/finalize", start.id),
                            Some(&c.enrollment_secret),
                            Some(&FinalizeRequest {
                                account_id: account_id.clone(),
                            }),
                        )
                        .await
                    {
                        Ok(_) => {
                            let own = client.own_device(c).await?;
                            c.device_id = Some(own.id);
                            storage.save(c)?;
                            report(Progress::Registered(own.id));
                            return Ok(());
                        }
                        Err(e) if e.retryable() && tokio::time::Instant::now() < deadline => {
                            wait_retry(deadline, attempt, e.retry_after.max(POLL_SECONDS)).await;
                            attempt = attempt.saturating_add(1);
                        }
                        Err(e) => return Err(e.into()),
                    }
                }
            }
            EnrollmentState::Pending => {
                tokio::time::sleep(std::time::Duration::from_secs(POLL_SECONDS)).await
            }
            _ => {
                storage.archive()?;
                bail!(
                    "Pairing ended ({:?}). Rerun connect to explicitly start again.",
                    v.state
                );
            }
        }
    }
}

/// Honor server throttling, but never extend a finite pairing wait indefinitely.
async fn wait_retry(deadline: tokio::time::Instant, attempt: u32, retry_after: u64) {
    let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
    let delay = std::time::Duration::from_secs(
        retry_seconds(attempt, super::jitter() as u8).max(retry_after),
    );
    tokio::time::sleep(delay.min(remaining)).await;
}
