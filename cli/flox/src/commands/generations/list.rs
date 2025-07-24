use std::fmt::Display;

use anyhow::Result;
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::generations::{
    AllGenerationsMetadata,
    GenerationId,
    SingleGenerationMetadata,
};
use tracing::instrument;

use super::try_get_generations_metadata;
use crate::commands::{EnvironmentSelect, environment_select};

#[derive(Bpaf, Debug, Clone)]
pub struct List {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,
}

impl List {
    #[instrument(name = "list", skip_all)]
    pub fn handle(self, flox: Flox) -> Result<()> {
        let env = self.environment.to_concrete_environment(&flox)?;
        let metadata = try_get_generations_metadata(&env)?;
        println!("{}", DisplayAllMetadata(&metadata));
        Ok(())
    }
}

struct DisplayMetadata<'m> {
    metadata: &'m SingleGenerationMetadata,
    id: &'m GenerationId,
    active: bool,
}
impl Display for DisplayMetadata<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.id)?;
        if self.active {
            write!(f, " (current)")?;
        }
        writeln!(f, ": {}", self.metadata.description)?;
        write!(f, "Created: {}", self.metadata.created)?;
        if let Some(last_active) = self.metadata.last_active {
            writeln!(f)?;
            write!(f, "Last Active: {last_active}")?;
        };
        Ok(())
    }
}

struct DisplayAllMetadata<'m>(&'m AllGenerationsMetadata);
impl Display for DisplayAllMetadata<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut iter = self.0.generations.iter().peekable();
        while let (Some((id, metadata)), peek) = (iter.next(), iter.peek()) {
            let next = DisplayMetadata {
                id,
                metadata,
                active: Some(id) == self.0.current_gen.as_ref(),
            };
            write!(f, "* {}", indent::indent_by(2, next.to_string()))?;
            if peek.is_some() {
                writeln!(f)?;
            }
        }
        Ok(())
    }
}
