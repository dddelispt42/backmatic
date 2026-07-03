use crate::config::types::HealthcheckConfig;
use crate::error::Result;
use crate::inject::BackmaticContext;

pub fn ping_success(ctx: &BackmaticContext, hc: &HealthcheckConfig) -> Result<()> {
    ctx.http.post_ping(&hc.url, &hc.uuid, None)?;
    log::debug!("healthcheck success ping {}", hc.uuid);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn ping_failure(
    ctx: &BackmaticContext,
    hc: &HealthcheckConfig,
    job_type: &str,
    comment: &str,
    attempts: u32,
    max_attempts: u32,
    last_error: &str,
    logfile: &str,
    dest: &str,
) -> Result<()> {
    let body = format!(
        "job_type={job_type}\ncomment={comment}\nattempts={attempts}/{max_attempts}\nlast_error={last_error}\nlogfile={logfile}\ndest={dest}"
    );
    ctx.http.post_fail(&hc.url, &hc.uuid, &body)?;
    log::error!("healthcheck failure ping {}: {}", hc.uuid, last_error);
    Ok(())
}
