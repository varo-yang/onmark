//! Deterministic explanation of solved and planned film facts.

use std::process::ExitCode;

use crate::arguments::InspectArgs;
use crate::check::Validation;
use crate::failure::CliError;

mod human;
mod json;

pub(super) struct InspectOutcome {
    validation: Validation,
    json: bool,
}

impl InspectOutcome {
    pub(super) fn write(self) -> ExitCode {
        let exit_code = if self.validation.inspection.is_some() {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        };
        let result = if self.json {
            json::write(&self.validation)
        } else {
            human::write(&self.validation)
        };
        result.map_or(ExitCode::FAILURE, |()| exit_code)
    }
}

pub(super) async fn run(args: InspectArgs, json: bool) -> Result<InspectOutcome, CliError> {
    let validation = crate::check::validate(args.validation).await?;
    Ok(InspectOutcome { validation, json })
}
