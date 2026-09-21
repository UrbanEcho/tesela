use super::*;

impl<A: Action> sealed::Sealed for A {}
impl<A: Action> Plan for A {
    type Output = A::Output;
    fn execute(
        self,
        runtime: &raw::Runtime,
        context: &crate::Context,
    ) -> Result<Execution<Self::Output>, Error> {
        let definition = A::definition();
        let execution = runtime.execute_action(
            context,
            raw::ActionRequest {
                action: definition.api_name,
                input: self.encode()?,
                run_id: None,
                expected_revision: None,
                preconditions: Vec::new(),
            },
        )?;
        if execution.output.status != "success" {
            return Err(Error::adapter(
                execution
                    .output
                    .error
                    .unwrap_or_else(|| "action failed".to_owned()),
            ));
        }
        Ok(Execution {
            value: A::decode(execution.output.output)?,
            receipt: execution.receipt,
            revision: None,
        })
    }
}
