use crate::adapters::calendar::evolution_db::EvolutionDbStore;
use crate::adapters::calendar::ics_files::IcsFilesStore;
use crate::adapters::wayland::hyprland::HyprlandDesktop;
use crate::application::calendar_service::CalendarService;
use crate::config::AppConfig;
use crate::error::Result;
use clap::{Args, Subcommand};

#[derive(Args, Debug)]
pub struct CalendarArgs {
    #[command(subcommand)]
    pub action: CalendarAction,
}

#[derive(Subcommand, Debug)]
pub enum CalendarAction {
    /// List upcoming expanded events as JSON
    List {
        /// Start date in YYYY-MM-DD format (defaults to today)
        #[arg(long = "from")]
        from_date: Option<String>,
        /// Number of days to include
        #[arg(long, default_value_t = 60)]
        days: i64,
    },
    /// Create a temporary .ics event and open it in the default calendar app
    Add {
        /// Date in YYYY-MM-DD format
        date: String,
        /// Start time in HH:MM format or "allday"
        time: String,
        /// Event summary/title
        title: Vec<String>,
    },
    /// Open the system default calendar application
    Open {
        /// Optional date in YYYY-MM-DD format
        date: Option<String>,
    },
}

pub fn run(args: CalendarArgs, cfg: &AppConfig) -> Result<()> {
    let ics_store = IcsFilesStore::new(cfg.ics_globs.clone());
    let evo_store = EvolutionDbStore::new(cfg.evolution_db_globs.clone());
    let desktop = HyprlandDesktop;
    let service = CalendarService::new(
        vec![&ics_store, &evo_store],
        &desktop,
        cfg.default_event_duration_mins,
    );

    match args.action {
        CalendarAction::List { from_date, days } => {
            let events = service.list_upcoming(from_date.as_deref(), days)?;
            println!("{}", serde_json::to_string(&events)?);
        }
        CalendarAction::Add { date, time, title } => {
            let summary = title.join(" ");
            service.add_event(&date, &time, &summary)?;
        }
        CalendarAction::Open { date } => {
            service.open_calendar(date.as_deref())?;
        }
    }
    Ok(())
}
