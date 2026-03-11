use crate::CommandOutput;

pub mod create;
pub mod delete;
pub mod fork;
pub mod list;
pub mod psql;
pub mod show;
pub mod start;
pub mod stop;

#[derive(serde::Serialize)]
pub enum Outputs {
    List(list::ListOutput),
    Create(create::CreateOutput),
    Psql(psql::PsqlOutput),
    Show(show::ShowOutput),
    Fork(fork::ForkOutput),
    Delete(delete::DeleteOutput),
    Start(start::StartOutput),
    Stop(stop::StopOutput),
}

impl CommandOutput for Outputs {
    fn to_text(&self) -> String {
        match self {
            Outputs::List(output) => output.to_text(),
            Outputs::Create(output) => output.to_text(),
            Outputs::Psql(output) => output.to_text(),
            Outputs::Show(output) => output.to_text(),
            Outputs::Fork(output) => output.to_text(),
            Outputs::Delete(output) => output.to_text(),
            Outputs::Start(output) => output.to_text(),
            Outputs::Stop(output) => output.to_text(),
        }
    }
}
