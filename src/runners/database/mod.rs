mod mysql;
mod postgres;

use crate::config::types::{DatabaseJob, JobId};
use crate::error::Result;
use crate::inject::BackmaticContext;
use crate::mount::resolve_destinations;

pub fn run(ctx: &BackmaticContext, job_id: &JobId, job: &DatabaseJob) -> Result<()> {
    let scope = job_id.scope_key();
    let (dests, _session) =
        resolve_destinations(ctx, &scope, &job.comment, &job.dest, &job.destmount)?;
    match job.engine {
        crate::config::types::DatabaseEngine::Mysql => mysql::run(ctx, job, &dests),
        crate::config::types::DatabaseEngine::Postgres => postgres::run(ctx, job, &dests),
    }
}
