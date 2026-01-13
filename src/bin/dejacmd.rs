use std::io::{self, BufRead, Read, Write};
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use colored::Colorize;
use sqlx::sqlite::SqliteConnectOptions;
use short_uuid::ShortUuid;
use chrono::TimeZone;
use indicatif::{ProgressBar, ProgressStyle};
use sqlx::{Row, Column};
use futures::stream::TryStreamExt;

use dejacmd::settings::Settings;
use dejacmd::{CREATE_INDEX_SQL, CREATE_TABLE_SQL, INSERT_HISTORY_SQL, connections, fix_placeholders, get_database };

#[derive(Parser)]
#[command(name = "dejacmd")]
#[command(about = "Command line history database")]
#[command(long_about =
r#""#)]
#[command(after_help =
r#"Command Aliases:
search = s or se or sea or sear
query = q or qu or que or quer
config = c or co or con or conf
import = i or im or imp
export = e or ex or exp"#)]

// #[command(name = "dejacmd", about = "Command line history database", author = "Donald Munro", version = "0.1.0", long_about = None)]
struct Cli
{
   #[command(subcommand)]
   command: Commands,
}

#[derive(Subcommand)]
enum Commands
{
   #[command(after_help =
   r#"Examples:
   dejacmd search "rsync -avz" -n 10
   dejacmd s -u "ls -al"
   dejacmd se  "df -h" -s 2024-03-01_13:00:00 -e 2024-03-31_13:00:00 "#)]
   #[command(aliases = ["s", "se", "sea", "sear", "searc"])]
   Search
   {
      #[arg(help = "Command line history search string filter")] // positional
      search_spec: Option<String>,

      #[arg(long="central", help = "Search central database if configured (defaults to local database). Applies to both search and query.")]
      is_central_search_query: bool,

      #[arg(short = 'n', long = "lines", default_value_t=25, help = "Number of lines to show from history")]
      number: u64,

      #[arg(short = 'i', long="no-case", help = "Case insensitive search")]
      is_ignore_case: bool,

      #[arg(short = 't', long="no-time", help = "Don't show timestamps in output")]
      is_not_show_time: bool,

      #[arg(short = 'u', long="unique", help = "Filter out duplicate commands in output (implies -t no timestamps)")]
      is_unique: bool,

      #[arg(short = 's', long="start", default_value = "",
         help = "Start timestamp for search in YYYY-MM-DD_HH:MM:SS format (if no time specified, assumes 00:00:00)")]
      start_time: Option<String>,

      #[arg(short = 'e', long="end", default_value = "",
         help = "End timestamp for search in YYYY-MM-DD_HH:MM:SS format (if no time specified, assumes 00:00:00)")]
      end_time: Option<String>,      
   },

   #[command(after_help =
   r#"Examples:
   dejacmd query "SELECT command, command_timestamp FROM history WHERE shell='bash' LIMIT 10"
   dejacmd query "SELECT DISTINCT shell FROM history"
   dejacmd query "SELECT COUNT(*) FROM history WHERE command LIKE '%docker%'"
   dejacmd query --central "SELECT * FROM history ORDER BY command_timestamp DESC LIMIT 5"

Note: If no query is provided, you will be prompted to enter one interactively."#)]
   #[command(aliases = ["q", "qu", "que", "quer"])]
   Query
   {
      #[arg(help = "Custom SQL query to execute against history database")] // positional
      sql: Option<String>,

      #[arg(long="central", help = "Query central database if configured (defaults to local database).")]
      is_central_query: bool,

      #[arg(short='D', long = "ddl",  help = "Show the DDL for the history table (for custom queries)")]
      is_show_ddl: bool,
   },

   #[command(aliases = ["c", "co", "con", "conf"])]
   Config
   {
      #[arg(short = 'L', long = "local-database", num_args = 0..=1, default_missing_value = "",
            help = r#"Get or set local database URL in settings file [default sqlite://~/.dejacmd.sqlite].
            When setting {{user}} and {{password}} can be used as placeholders for username and password respectively (use -u and -p options for user and password).
            Password will be encrypted in the settings file.
            Use ~ for the user home directory if using SQLite which will be fully expanded when written.
            Examples: dejacmd config -L "sqlite://~/Documents/dejacmd.sqlite
            dejacmd config -L "postgresql://{{user}}:{{password}}@localhost/myowndb" -u postgres -p"pAssword" "#)]
      local_url: Option<String>,

      #[arg(short = 'C', long = "central-database", num_args = 0..=1, default_missing_value = "",
            help = r#"Get or set Central database URL in settings file.
            When setting {{user}} and {{password}} can be used as placeholders for username and password respectively (use -u and -p options for user and password).
            Password will be encrypted in the settings file.
            Use ~ for the user home directory if using SQLite which will be fully expanded when written.
            Examples: dejacmd config -C "postgresql://{{user}}:{{password}}@localhost/dejacmd" -u postgres -p
            dejacmd config -C "mysql://{{user}}:{{password}}@localhost/dejacmd" -u me -p
            dejacmd config -C "sqlite:///home/share/history/dejacmd.sqlite"#)]
      central_url: Option<String>,

      #[arg(short = 'u', long = "user", default_value = "",
            help = "Database user to use with -L or -C database URLs for databases where authentication is required.")]
      user: String,

      #[arg(short = 'p', long = "password", num_args = 0..=1, default_missing_value = "",
            help = r#"Database password to use with -L or -C database URLs for databases where authentication is required.).
            If flag is present but no value provided, will prompt for password"#)]
      password: Option<String>,

      #[arg(short = 's', long = "show", help = "Show password when entering from console")]
      is_show_password: bool,
   },

   #[command(aliases = ["i", "im", "imp"])]
   Import
   {
      #[arg(help = "Shell history file e.g .bash_history or recent SQLite database e.g ~/.recent.db")] // positional
      shell_history_file: String,

      #[arg(short = 'T', long = "truncate", help = "Truncate history table before importing")]
      is_truncate: bool
   },

   #[command(aliases = ["e", "ex", "exp"])]
   Export
   {
      #[arg(help = "Export to a bash or zsh history file")] // positional
      export_history_file: String,

      #[arg(short = 'E', long = "format", default_value="bash", help = "Export format: bash or zsh [bash]")]
      export_history_format: String,

      #[arg(short = 'F', long = "from-central", help = "Export history from central database if configured (defaults to local database)")]
      is_central_export: bool,
   }
}

#[tokio::main]
async fn main()
//------------
{
   let args = Cli::parse();
   let mut settings = Settings::new();
   settings = settings.get_settings_or_default();

   match args.command
   {
      Commands::Search { search_spec, number, is_ignore_case, is_central_search_query, is_not_show_time, is_unique,
         start_time, end_time } =>
      {         
         let spec: String;
         if search_spec.is_none()
         {
            spec = "".to_string();
         }
         else
         {
            spec = search_spec.clone().unwrap();
         }
         let is_time = ! is_not_show_time && !is_unique;
         if let Err(e) = search(&spec, number, is_ignore_case, is_central_search_query, is_time, is_unique,
            start_time, end_time, &settings).await
         {
            eprintln!("{}: {}", "Error searching history".bright_red(), e);
         }
         return;
      },

      Commands::Config { local_url, central_url, user, password, is_show_password } =>
      {
         let password_opt = password.clone();
         if local_url.is_some()
         {
            handle_database_config(&mut settings, local_url, &user, password_opt, is_show_password, true);
         }
         else if central_url.is_some()
         {
            handle_database_config(&mut settings, central_url, &user, password.clone(), is_show_password, false);
         }
         return;
      },

      Commands::Import { shell_history_file, is_truncate } =>
      {
         if !shell_history_file.is_empty()
         {
            if let Err(e) = import_history(&shell_history_file, is_truncate, &settings).await
            {
               eprintln!("{}: {}", "Error importing shell history".bright_red(), e);
            }
            return;
         }
      }
      Commands::Export { export_history_file, export_history_format, is_central_export } =>
      {
         if export_history_file != ""
         {
            if let Err(e) = export_shell_history(&export_history_file, export_history_format, is_central_export,
                &settings).await
            {
               eprintln!("{}: {}", "Error export shell history".bright_red(), e);
            }
            return;
         }
      },

      Commands::Query { sql, is_central_query, is_show_ddl  } =>
      {
         if is_show_ddl
         {
            println!("{}\n{}", CREATE_TABLE_SQL, CREATE_INDEX_SQL);
            return;
         }
         let query_str: String;
         if sql.is_none() || sql.as_ref().unwrap().is_empty()
         {
            // Prompt user to enter SQL query
            print!("{}", "Enter SQL query: ".bright_cyan());
            io::stdout().flush().unwrap();
            let mut input = String::new();
            io::stdin().read_line(&mut input).expect("Failed to read query");
            query_str = input.trim().to_string();
         }
         else
         {
            query_str = sql.clone().unwrap();
         }

         if query_str.is_empty()
         {
            eprintln!("{}", "No query provided".bright_red());
            return;
         }

         if let Err(e) = query(&query_str, is_central_query, &settings).await
         {
            eprintln!("{}: {}", "Error executing query".bright_red(), e);
         }
         return;
      },
   }
}

fn parse_time_range(start_time: &Option<String>, end_time: &Option<String>) -> Result<(Option<String>, Option<String>), String>
//----------------------------------------------------------------------------------------------------------------------------------------------
{
   let start_datetime = if let Some(start) = start_time
   {
      if start.trim().is_empty()
      {
         None
      }
      else
      {
         Some(parse_datetime_string(start)?)
      }
   }
   else
   {
      None
   };

   let end_datetime = if let Some(end) = end_time
   {
      if end.trim().is_empty()
      {
         if start_datetime.is_some()
         {
            // Default to current time if start is specified but end is not
            let now = chrono::Utc::now();
            Some(now.format("%Y-%m-%d %H:%M:%S").to_string())
         }
         else
         {
            None
         }
      }
      else
      {
         Some(parse_datetime_string(end)?)
      }
   }
   else if start_datetime.is_some()
   {
      // Default to current time if start is specified but end is not
      let now = chrono::Utc::now();
      Some(now.format("%Y-%m-%d %H:%M:%S").to_string())
   }
   else
   {
      None
   };

   Ok((start_datetime, end_datetime))
}

fn parse_datetime_string(datetime_str: &str) -> Result<String, String>
//---------------------------------------------------------------------
{
   let datetime_str = datetime_str.trim();

   // Check if time is included (contains underscore or colon)
   if datetime_str.contains('_') || datetime_str.matches(':').count() >= 1
   {
      // Full datetime format: YYYY-MM-DD_HH:MM:SS or YYYY-MM-DD HH:MM:SS
      let normalized = datetime_str.replace('_', " ");

      // Try to parse to validate the format
      match chrono::NaiveDateTime::parse_from_str(&normalized, "%Y-%m-%d %H:%M:%S")
      {
         Ok(_) => Ok(normalized),
         Err(_) =>
         {
            // Try parsing with just date and time without seconds
            if normalized.matches(':').count() == 1
            {
               match chrono::NaiveDateTime::parse_from_str(&format!("{} 00", normalized), "%Y-%m-%d %H:%M:%S")
               {
                  Ok(dt) => Ok(dt.format("%Y-%m-%d %H:%M:%S").to_string()),
                  Err(e) => Err(format!("Invalid datetime format '{}'. Expected YYYY-MM-DD_HH:MM:SS or YYYY-MM-DD_HH:MM. Error: {}", datetime_str, e))
               }
            }
            else
            {
               Err(format!("Invalid datetime format '{}'. Expected YYYY-MM-DD_HH:MM:SS or YYYY-MM-DD_HH:MM", datetime_str))
            }
         }
      }
   }
   else
   {
      // Date only format: YYYY-MM-DD, assume 00:00:00
      match chrono::NaiveDate::parse_from_str(datetime_str, "%Y-%m-%d")
      {
         Ok(date) =>
         {
            let datetime = date.and_hms_opt(0, 0, 0).ok_or_else(|| "Invalid date".to_string())?;
            Ok(datetime.format("%Y-%m-%d %H:%M:%S").to_string())
         }
         Err(e) => Err(format!("Invalid date format '{}'. Expected YYYY-MM-DD. Error: {}", datetime_str, e))
      }
   }
}

async fn import_history(shell_history_file: &str, is_truncate: bool, settings: &Settings) -> Result<(), String>
//---------------------------------------------------------------------
{
   let mut file = std::fs::File::open(shell_history_file).map_err(|e| e.to_string())?;
   let mut buffer = [0u8; 16];

   let is_sqlite = match file.read_exact(&mut buffer)
   {
      Ok(_) => &buffer == b"SQLite format 3\0",
      Err(_) => false,
   };
   sqlx::any::install_default_drivers();

   if is_sqlite
   {
      import_sqlite_history(shell_history_file, is_truncate, settings).await
   }
   else
   {
      import_shell_history(shell_history_file, is_truncate, settings).await
   }
}

pub async fn search(spec: &str, mut no: u64, is_ignore_case: bool, is_central: bool, is_show_time: bool, is_unique: bool,
   start_time: Option<String>, end_time: Option<String>, settings: &Settings) -> Result<(), String>
//------------------------------------------------------------------------------------------------------
{
   // Validate date parameters
   if end_time.is_some() && end_time.as_ref().unwrap() != "" && (start_time.is_none() || start_time.as_ref().unwrap() == "")
   {
      return Err("End time cannot be specified without a start time".to_string());
   }
   if no == 0
   {
      no = 25;
   }
   let (url, user, password): (String, String, String);
   if is_central
    {
       url = settings.get_central_database_url();
       (user, password) = match settings.get_credentials(false)
       {
          Ok((u, p)) => (u, p),
          Err(_) => ("".to_string(), "".to_string())
       };
    }
    else
    {
       url = settings.get_local_database_url();
       (user, password) = match settings.get_credentials(true)
       {
          Ok((u, p)) => (u, p),
          Err(_) => ("".to_string(), "".to_string())
       };
    }
    if url.trim().is_empty()
    {
       return Err("No database URL configured".to_string());
    }
    sqlx::any::install_default_drivers();
    let (pool_opt, scheme) = match get_database(&url, &user, &password).await
    {
       Ok((p, s)) => (p, s),
       Err(e) => return Err(format!("Error connecting to {} database: {}", if is_central { "central" } else { "local" }, e)),
    };
    if let Some(pool) = pool_opt
    {
       let term= if spec.trim().is_empty() {"".to_string()} else { format!("%{}%", spec) };
       let select = format!("{} {} command ",
          if is_unique { "DISTINCT" } else { "" },
          if is_show_time { "command_timestamp," } else { "" });
       let from = "history";

       // Parse and format start and end times
       let (start_datetime, end_datetime) = parse_time_range(&start_time, &end_time)?;

       // Build WHERE clause
       let mut where_conditions = Vec::new();

       if !spec.trim().is_empty()
       {
          if is_ignore_case
          {
             where_conditions.push("LOWER(command) LIKE LOWER(?)".to_string());
          } else {
             where_conditions.push("command LIKE ?".to_string());
          }
       }

       if start_datetime.is_some()
       {
          where_conditions.push("command_timestamp >= ?".to_string());
       }

       if end_datetime.is_some()
       {
          where_conditions.push("command_timestamp <= ?".to_string());
       }

       let wher = if where_conditions.is_empty()
       {
          "1=1".to_string()
       }
       else
       {
          where_conditions.join(" AND ")
       };

       let order = "command_timestamp DESC";
       let limit = if no > 0 { format!("LIMIT {}", no) } else { "".to_string() };
       let sql = format!("SELECT {} FROM {} WHERE {} ORDER BY {} {}", select, from, wher, order, limit);
       let query = fix_placeholders(&sql, &scheme);
       //println!("{}: {} with {}", "Executing query".bright_cyan(), query.bright_white(), term.bright_white());
       let mut query_builder = sqlx::query(&query);

       if !term.is_empty()
       {
          query_builder = query_builder.bind(&term);
       }

       if let Some(ref start) = start_datetime
       {
          query_builder = query_builder.bind(start);
       }

       if let Some(ref end) = end_datetime
       {
          query_builder = query_builder.bind(end);
       }

       let rows = query_builder
            // .bind(no as i64)
            .fetch(&pool);
         let mut _count = 0;
         let mut _errors = 0;
         tokio::pin!(rows);
         while let Some(row) = rows.try_next().await
                               .map_err(|e| format!("{} with {} [{}]", query, term, e.to_string().red()))?
         {
            let date: String = if is_show_time { row.get("command_timestamp") } else { "".to_string() };
            let command: String = row.get("command");
            let mut highlighted = String::new();
            let search_term = if is_ignore_case { spec.to_lowercase() } else { spec.to_string() };
            let key = if is_ignore_case { command.to_lowercase() } else { command.clone() };

            // We only attempt highlighting if strings are byte-length compatible to avoid Unicode index issues
            if !spec.is_empty() && key.len() == command.len()
            {
               let mut last_idx = 0;
               for (idx, m) in key.match_indices(&search_term)
               {
                  highlighted.push_str(&command[last_idx..idx]);
                  highlighted.push_str(&format!("{}", command[idx..idx + m.len()].red().bold()));
                  last_idx = idx + m.len();
               }
               highlighted.push_str(&command[last_idx..]);
            }
            else
            {
               highlighted = command;
            }
            println!("{}  {}", date.bright_blue(), highlighted);
            _count += 1;
         }
    }
    else
    {
         return Err("Failed to establish database connection".to_string());
    }
    Ok(())
}

pub async fn query(sql: &str, is_central: bool, settings: &Settings) -> Result<(), String>
//----------------------------------------------------------------------------------------
{
   let (url, user, password): (String, String, String);
   if is_central
   {
      url = settings.get_central_database_url();
      (user, password) = match settings.get_credentials(false)
      {
         Ok((u, p)) => (u, p),
         Err(_) => ("".to_string(), "".to_string())
      };
   }
   else
   {
      url = settings.get_local_database_url();
      (user, password) = match settings.get_credentials(true)
      {
         Ok((u, p)) => (u, p),
         Err(_) => ("".to_string(), "".to_string())
      };
   }
   if url.trim().is_empty()
   {
      return Err("No database URL configured".to_string());
   }
   sqlx::any::install_default_drivers();
   let (pool_opt, scheme) = match get_database(&url, &user, &password).await
   {
      Ok((p, s)) => (p, s),
      Err(e) => return Err(format!("Error connecting to {} database: {}", if is_central { "central" } else { "local" }, e)),
   };

   if let Some(pool) = pool_opt
   {
      // Fix placeholders for PostgreSQL if needed
      let fixed_sql = fix_placeholders(sql, &scheme);

      // Execute the query
      let rows = sqlx::query(&fixed_sql)
         .fetch(&pool);

      tokio::pin!(rows);
      let mut count = 0;
      let mut is_first_row = true;

      while let Some(row) = rows.try_next().await
         .map_err(|e| format!("Error executing query: {}", e.to_string().red()))?
      {
         // Print column headers on first row
         if is_first_row
         {
            let columns = row.columns();
            let header: Vec<String> = columns.iter()
               .map(|col| col.name().to_string())
               .collect();
            println!("{}", header.join(" | ").bright_cyan().bold());
            println!("{}", "-".repeat(header.join(" | ").len()).bright_black());
            is_first_row = false;
         }

         // Print row data
         let columns = row.columns();
         let mut values = Vec::new();

         for col in columns
         {
            // Try to get the value as different types
            let value = if let Ok(v) = row.try_get::<String, _>(col.name())
            {
               v
            }
            else if let Ok(v) = row.try_get::<i64, _>(col.name())
            {
               v.to_string()
            }
            else if let Ok(v) = row.try_get::<i32, _>(col.name())
            {
               v.to_string()
            }
            else if let Ok(v) = row.try_get::<f64, _>(col.name())
            {
               v.to_string()
            }
            else if let Ok(v) = row.try_get::<bool, _>(col.name())
            {
               v.to_string()
            }
            else
            {
               "NULL".to_string()
            };
            values.push(value);
         }

         println!("{}", values.join(" | "));
         count += 1;
      }

      if count == 0
      {
         println!("{}", "No rows returned".yellow());
      }
      else
      {
         println!("\n{} {} returned", count.to_string().bright_white(), if count == 1 { "row" } else { "rows" });
      }
   }
   else
   {
      return Err("Failed to establish database connection".to_string());
   }
   Ok(())
}

async fn import_sqlite_history(sqlite_history_file: &str, is_truncate: bool, settings: &Settings) -> Result<(), String>
//-------------------------------------------------------------------------------------------------------------------
{
   let options = SqliteConnectOptions::new().filename(sqlite_history_file);
   let in_pool = sqlx::SqlitePool::connect_with(options).await
      .map_err(|e| format!("Error connecting to recent SQLite history file {}: {}", sqlite_history_file, e))?;

   /*
    * CREATE TABLE commands (
                command_dt timestamp,
                command text,
                pid int,
                return_val int,
                pwd text,
                session text,
                json_data json
            )
    */

   let rows = sqlx::query("SELECT COUNT(*) FROM commands")
         .fetch_all(&in_pool)
         .await
         .map_err(|e| format!("Error querying history count from recent SQLite database {}: {}", sqlite_history_file, e))?;
   let total_count: i64 = rows[0].get(0);
   if total_count == 0
   {
      return Err("Recent SQLite history file contains no history entries".to_string());
   }

   let (local_pool_opt, local_scheme, central_pool_opt, central_scheme) = match connections(settings, true, is_truncate).await
   {
      Ok(c) => c,
      Err(e) => return Err(format!("Error connecting to database: {}", e)),
   };

   println!("{}", "Importing SQLite shell history...".bright_cyan());
   let pb = ProgressBar::new(total_count as u64);
      pb.set_style(
         ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({percent}%) {msg}")
            .unwrap()
            .progress_chars("#>-")
      );

   let rows = sqlx::query("SELECT command_dt, command, return_val, pwd FROM commands")
         .fetch(&in_pool);
   let mut count = 0;
   let mut errors = 0;
   tokio::pin!(rows);
   while let Some(row) = rows.try_next().await.map_err(|e| e.to_string())?
   {
      let command: String = row.get("command");
      let command_dt: String = row.get("command_dt");
      let status: i64 = row.get("return_val");
      let pwd: String = row.get("pwd");

      let dt = chrono::NaiveDateTime::parse_from_str(&command_dt, "%Y-%m-%d %H:%M:%S")
         .map_err(|e| format!("Error parsing timestamp '{}': {}", command_dt, e))?;
      let timestamp = dt.and_utc().timestamp();

      if let Err(e) = insert_history_entry(&local_pool_opt, &central_pool_opt, &local_scheme, &central_scheme,
         &command, &pwd, timestamp, "bash", status).await
      {
         pb.println(format!("{} {}: {}", "Error inserting sqlite history entry".yellow(), command.red(), e));
         errors += 1;
      }
      else
      {
         count += 1;
      }
      pb.inc(1);
   }
   pb.finish_with_message(format!("{} {} commands imported", "Successfully".bright_green(), count.to_string().bright_white()));
   if errors > 0
   {
      println!("{} {} errors encountered", "Warning:".yellow(), errors.to_string().bright_white());
   }
   Ok(())
}

async fn import_shell_history(shell_history_file: &str, is_truncate: bool, settings: &Settings) -> Result<(), String>
//---------------------------------------------------------------------
{
   let line_count = io::BufReader::new(std::fs::File::open(shell_history_file).map_err(|e| e.to_string())?)
      .lines()
      .count() as u64;
   if line_count == 0
   {
      return Err("Shell history file is empty".to_string());
   }

   let fd = match std::fs::File::open(shell_history_file)
   {
      Ok(f) => f,
      Err(e) => return Err(format!("Failed to open shell history file: {}", e)),
   };

   let (local_pool_opt, local_scheme, central_pool_opt, central_scheme) = match connections(settings, true, is_truncate).await
   {
      Ok(c) => c,
      Err(e) => return Err(format!("Error connecting to database: {}", e)),
   };

   println!("{}", "Importing shell history...".bright_cyan());

   // Create progress bar
   let pb = ProgressBar::new(line_count);
   pb.set_style(
      ProgressStyle::default_bar()
         .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({percent}%) {msg}")
         .unwrap()
         .progress_chars("#>-")
   );


   // Parse and import history
   let reader = io::BufReader::new(fd);
   let mut lines = reader.lines().peekable();
   let mut count = 0;
   let mut errors = 0;
   let mut lineno = 1;

   while let Some(line_result) = lines.next()
   {
      let line = match line_result
      {
         Ok(l) => l,
         Err(e) =>
         {
            pb.println(format!("{} {}: {}", "Error reading line".yellow(), lineno, e));
            errors += 1;
            lineno += 1;
            pb.inc(1);
            continue;
         }
      };

      if line.trim().is_empty()
      {
         lineno += 1;
         pb.inc(1);
         continue;
      }

      if let Some(entry) = parse_zsh_format(&line)
      {
         if entry.command.is_empty()
         {
            lineno += 1;
            pb.inc(1);
            continue;
         }
         if entry.command.starts_with('#') && entry.command.len() == 11 //got some eg ": 1768106083:0;#1768105585" ????
         {
            lineno += 1;
            pb.inc(1);
            continue;
         }
         if let Err(e) = insert_history_entry(&local_pool_opt, &central_pool_opt, &local_scheme, &central_scheme,
            &entry.command, "", entry.timestamp, "zsh", -1).await
         {
            pb.println(format!("{} {}: {}", "Error inserting zsh history entry".yellow(), line.red(), e));
            errors += 1;
            lineno += 1;
         }
         else
         {
            count += 1;
            lineno += 1;
         }
         pb.inc(1);
         continue;
      }

      // Check for bash timestamp comment format: "#<timestamp>"
      if line.trim().starts_with('#')
      {
         if let Ok(timestamp) = line[1..].trim().parse::<i64>()
         {
            // Peek at next line to get the command
            if let Some(Ok(command)) = lines.peek()
            {
               if !command.is_empty() && !command.starts_with('#')
               {
                  if let Err(e) = insert_history_entry(&local_pool_opt, &central_pool_opt, &local_scheme, &central_scheme, command,
                     "", timestamp, "bash", -1).await
                  {
                     pb.println(format!("{} {}: {}", "Error inserting bash entry".yellow(), line.red(), e));
                     errors += 1;
                     lineno += 1;
                  }
                  else
                  {
                     count += 1;
                     lineno += 1;
                  }
                  lines.next(); // Consume the peeked line
                  pb.inc(2); // Increment by 2 (timestamp line + command line)
                  continue;
               }
            }
         }
      }

      // Single line bash format (no timestamp)
      if !line.starts_with('#')
      {
         let timestamp = 0; //chrono::Utc::now().timestamp();
         if let Err(e) = insert_history_entry(&local_pool_opt, &central_pool_opt, &local_scheme, &central_scheme, &line,
               "", timestamp, "bash", -1).await
         {
            pb.println(format!("{} {}: {}", "Error inserting bash entry (no timestamp)".yellow(), line.red(), e));
            errors += 1;
            lineno += 1;
         }
         else
         {
            count += 1;
            lineno += 1;
         }
         pb.inc(1);
      }
   }

   // Finish progress bar
   pb.finish_with_message(format!("{} {} commands imported", "Successfully".bright_green(), count.to_string().bright_white()));

   if errors > 0
   {
      println!("{} {} errors encountered", "Warning:".yellow(), errors.to_string().bright_white());
   }

   Ok(())
}


async fn export_shell_history(export_file: &str, format: String, use_central: bool, settings: &Settings) -> Result<(), String>
//------------------------------------------------------------------------------------------------------------------------------
{
   println!("{}", format!("Exporting shell history to {}...", export_file).bright_cyan());

   sqlx::any::install_default_drivers();

   let db_url = if use_central
   {
      settings.get_central_database_url()
   } else
   {
      settings.get_local_database_url()
   };

   if db_url.trim().is_empty() {
      return Err(format!("No {} database URL configured", if use_central { "central" } else { "local" }));
   }

   let (user, password) = match settings.get_credentials(use_central)
   {
      Ok((u, p)) => (u, p),
      Err(_) => ("".to_string(), "".to_string())
   };

   let (pool_opt, _scheme) = match get_database(&db_url, &user, &password).await
   {
      Ok((p, s)) => (p, s),
      Err(e) => return Err(format!("Error connecting to database: {}", e)),
   };

   let pool = match pool_opt
   {
      Some(p) => p,
      None => return Err("Failed to establish database connection".to_string()),
   };

   // First, get the count for the progress bar
   let count_result = sqlx::query("SELECT COUNT(*) as count FROM history")
      .fetch_one(&pool)
      .await
      .map_err(|e| format!("Error querying history count: {}", e))?;
   let total_count: i64 = count_result.get("count");

   if total_count == 0 {
      println!("{}", "No history entries found to export".yellow());
      return Ok(());
   }

   // Create progress bar
   let pb = ProgressBar::new(total_count as u64);
   pb.set_style(
      ProgressStyle::default_bar()
         .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({percent}%) {msg}")
         .unwrap()
         .progress_chars("#>-")
   );

   // Open output file for writing
   let mut file = std::fs::File::create(export_file)
      .map_err(|e| format!("Failed to create export file: {}", e))?;

   let format_lower = format.to_lowercase();
   let mut exported_count = 0;

   // Stream rows instead of loading all at once
   let rows = sqlx::query("SELECT command, command_timestamp FROM history ORDER BY command_timestamp")
      .fetch(&pool);
   tokio::pin!(rows);

   while let Some(row) = rows.try_next().await.map_err(|e| format!("Error fetching row: {}", e))? {
      let command: String = row.get("command");
      let timestamp_str: String = row.get("command_timestamp");

      // Parse timestamp string to Unix timestamp
      // Format: "YYYY-MM-DD HH:MM:SS"
      let timestamp = chrono::NaiveDateTime::parse_from_str(&timestamp_str, "%Y-%m-%d %H:%M:%S")
         .map_err(|e| format!("Error parsing timestamp '{}': {}", timestamp_str, e))?
         .and_utc()
         .timestamp();

      // Write in appropriate format
      if format_lower == "zsh" {
         // Zsh format: ": timestamp:0;command\n"
         writeln!(file, ": {}:0;{}", timestamp, command)
            .map_err(|e| format!("Error writing to file: {}", e))?;
      } else {
         // Bash format (default): "#timestamp\ncommand\n"
         writeln!(file, "#{}", timestamp)
            .map_err(|e| format!("Error writing to file: {}", e))?;
         writeln!(file, "{}", command)
            .map_err(|e| format!("Error writing to file: {}", e))?;
      }

      exported_count += 1;
      pb.inc(1);
   }

   pb.finish_with_message(format!("{} {} commands exported to {}",
      "Successfully".bright_green(),
      exported_count.to_string().bright_white(),
      export_file.bright_white()));

   Ok(())
}

struct ZshEntry
{
   timestamp: i64,
   command: String,
}

fn parse_zsh_format(line: &str) -> Option<ZshEntry>
{
   if !line.starts_with(": ")
   {
      return None;
   }

   let rest = &line[2..]; // Skip ": "
   let parts: Vec<&str> = rest.splitn(2, ';').collect();

   if parts.len() != 2
   {
      return None;
   }

   let time_parts: Vec<&str> = parts[0].split(':').collect();
   if time_parts.len() != 2
   {
      return None;
   }

   let timestamp = time_parts[0].parse::<i64>().ok()?;
   let command = parts[1].to_string();

   Some(ZshEntry {
      timestamp,
      command,
   })
}

async fn insert_history_entry( local_pool_opt: &Option<sqlx::Pool<sqlx::Any>>,
   central_pool_opt: &Option<sqlx::Pool<sqlx::Any>>,
   local_scheme: &str, central_scheme: &str, command: &str, pwd: &str,
   timestamp: i64, shell_name: &str, status: i64 ) -> Result<(), String>
//-------------------------------------------------------------------------------
{
   let id = ShortUuid::generate();

   let dt = chrono::Utc.timestamp_opt(timestamp, 0)
      .single()
      .ok_or_else(|| "Invalid timestamp".to_string())?;
   let command_date = dt.format("%Y-%m-%d %H:%M:%S").to_string();

   let cwd : PathBuf; // = std::env::current_dir().unwrap_or_default();
   if pwd.trim().is_empty()
   {
      cwd = std::env::current_dir().unwrap_or_default();
   }
   else
   {
      cwd = PathBuf::from(pwd);
   }
   let mut user: String = "".to_string();
   if cfg!(target_os = "windows")
   {
      user = std::env::var("USERNAME").unwrap_or("".to_string());
   }
   else
   {
      use nix::unistd::{getuid, User, Uid};
      let uid: Uid = getuid();
      if let Ok(user_info) = User::from_uid(uid) && let Some(u) = user_info
      {
         if !u.name.is_empty()
         {
            user = u.name;
         }
      }
   }
   let ip = match localip::get_local_ip()
   {
      Ok(i) => i.to_string(),
      Err(_) => "".to_string()
   };

   let local_sql = fix_placeholders(INSERT_HISTORY_SQL, local_scheme);
   let central_sql = fix_placeholders(INSERT_HISTORY_SQL, central_scheme);

   let local_insert = async
   {
      if let Some(local_pool) = local_pool_opt
      {
         let result = sqlx::query(&local_sql)
            .bind(id.to_string())
            .bind(&command_date)
            .bind(cwd.display().to_string())
            .bind(shell_name)
            .bind(None::<i64>) // user_id
            .bind(user.clone())
            .bind(ip.clone()) // ip
            .bind(status) // exit_status
            .bind(command)
            .execute(local_pool)
            .await;
         result
      }
      else
      {
         Ok(sqlx::any::AnyQueryResult::default())
      }
   };
   let central_insert = async
   {
      if let Some(central_pool) = central_pool_opt
      {
         let result = sqlx::query(&central_sql)
            .bind(id.to_string())
            .bind(&command_date)
            .bind(cwd.display().to_string())
            .bind(shell_name)
            .bind(None::<i64>) // user_id
            .bind(user.clone())
            .bind(ip.clone()) // ip
            .bind(None::<i64>) // exit_status
            .bind(command)
            .execute(central_pool)
            .await;
         result
      }
      else
      {
         Ok(sqlx::any::AnyQueryResult::default())
      }
   };
   let (local_result, central_result) = tokio::join!(local_insert, central_insert);
   if local_result.is_err()
   {
      let values = format!("VALUES ( {}, {}, {}, {}, {}, {}, {}, {}, {} )",
               id, command_date.clone(), cwd.display(), shell_name, -1, user.clone(),
               ip.clone(), 0, command );
      return Err(format!("{}: [{}]\n{} {}", "Error inserting command into local history database:".red(), local_result.err().unwrap().to_string().bright_red(),
                  local_sql, values));
   }
   if central_result.is_err()
   {
      let values = format!("VALUES ( {}, {}, {}, {}, {}, {}, {}, {}, {} )",
               id, command_date.clone(), cwd.display(), shell_name, -1, user.clone(),
               ip.clone(), 0, command );
      return Err(format!("{}: [{}]\n{} {}", "Error inserting command into central history database:".red(), central_result.err().unwrap().to_string().bright_red(),
                  local_sql, values));
   }
   Ok(())
}



fn handle_database_config( settings: &mut Settings, url: Option<String>, user: &str, password: Option<String>,
   show_password: bool, is_local: bool )
//---------------------------------------------------------------------------------------------------------
{
   match url
   {
       Some(url_value) if url_value.is_empty() || url_value == "true" =>
       {
           // Display current settings
           display_database_settings(settings, is_local);
       }
       Some(url_value) =>
       {
           // Set mode - update the database URL and credentials
           set_database_settings(settings, &url_value, user, password, show_password, is_local);
       }
       None => {
           // Display current settings
           display_database_settings(settings, is_local);
       }
   }
}

fn display_database_settings(settings: &Settings, is_local: bool)
//-----------------------------------------------------------------
{
   let db_type = if is_local { "Local" } else { "Central" };
   let url = if is_local {
       settings.get_local_database_url()
   } else {
       settings.get_central_database_url()
   };

   println!("{} Database Configuration:", db_type.bright_cyan());
   println!("  URL: {}", url.bright_white());

   match settings.get_credentials(is_local) {
       Ok((user, _password)) => {
           if !user.is_empty() {
               println!("  User: {}", user.bright_white());
           }
       }
       Err(e) => {
           eprintln!("  {}: {}", "Error reading credentials".bright_red(), e);
       }
   }
}

fn set_database_settings(
   settings: &mut Settings,
   url: &str,
   user: &str,
   password: Option<String>,
   show_password: bool,
   is_local: bool,
)
{
   let db_type = if is_local { "Local" } else { "Central" };

   // Expand ~ in the URL if it's a SQLite URL
   let expanded_url = expand_tilde_in_url(url);

   // Set the database URL
   if let Err(e) = settings.set_database_url(&expanded_url, is_local) {
       eprintln!("{}: {}", format!("Error setting {} database URL", db_type).bright_red(), e);
       return;
   }

   // Handle user and password if provided
   if !user.is_empty() || password.is_some() {
       let pwd = match password {
           Some(p) if p.is_empty() => {
               // Password flag was provided but no value - prompt for it
               prompt_for_password(show_password)
           }
           Some(p) => p,
           None => String::new(),
       };

       if !user.is_empty() && !pwd.is_empty() {
           if let Err(e) = settings.set_user_password(user, &pwd, is_local) {
               eprintln!("{}: {}", format!("Error setting {} credentials", db_type).bright_red(), e);
               return;
           }
       } else if !user.is_empty() {
           if let Err(e) = settings.set_user(user, is_local) {
               eprintln!("{}: {}", format!("Error setting {} user", db_type).bright_red(), e);
               return;
           }
       } else if !pwd.is_empty() {
           if let Err(e) = settings.set_password(&pwd, is_local) {
               eprintln!("{}: {}", format!("Error setting {} password", db_type).bright_red(), e);
               return;
           }
       }
   }

   println!("{}", format!("{} database configuration updated successfully", db_type).bright_green());
   display_database_settings(settings, is_local);
}

fn prompt_for_password(show_password: bool) -> String
{
   print!("Enter password: ");
   io::stdout().flush().unwrap();

   if show_password {
       // Read password with echo enabled
       let mut password = String::new();
       io::stdin().read_line(&mut password).expect("Failed to read password");
       password.trim().to_string()
   } else {
       // Read password without echo using rpassword crate
       match rpassword::read_password() {
           Ok(pwd) => pwd,
           Err(e) => {
               eprintln!("{}: {}", "Error reading password".bright_red(), e);
               String::new()
           }
       }
   }
}

fn expand_tilde_in_url(url: &str) -> String
{
   if url.contains("~/") {
       let home_dir = Settings::get_home_dir();
       url.replace("~/", &format!("{}/", home_dir.display()))
   } else if url.starts_with("~") {
       let home_dir = Settings::get_home_dir();
       url.replacen("~", &home_dir.display().to_string(), 1)
   } else {
       url.to_string()
   }
}

#[cfg(test)]
mod tests
{
    use super::*;
    use sqlx::Row;

    fn create_test_settings() -> Settings
    {
        // Create a Settings instance for testing without file I/O
        // Use a temporary SQLite file database for testing
        // (in-memory creates a new DB on each connection)
        use std::time::{SystemTime, UNIX_EPOCH};

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let test_db = format!("sqlite:///tmp/dejacmd_test_{}.db", timestamp);
        Settings::new_for_test(&test_db, "")
    }

    fn cleanup_test_db(settings: &Settings) {
        // Extract the file path from the database URL
        let url = settings.get_local_database_url();
        if let Some(path_start) = url.find("///") {
            let path = &url[path_start + 2..];
            if let Some(query_start) = path.find('?') {
                let file_path = &path[..query_start];
                let _ = std::fs::remove_file(file_path);
            } else {
                let _ = std::fs::remove_file(path);
            }
        }
    }

    async fn count_history_entries(pool: &sqlx::Pool<sqlx::Any>) -> i64
    {
        sqlx::query("SELECT COUNT(*) as count FROM history")
            .fetch_one(pool)
            .await
            .unwrap()
            .get("count")
    }

    async fn get_commands(pool: &sqlx::Pool<sqlx::Any>) -> Vec<String>
    {
        let rows = sqlx::query("SELECT command FROM history ORDER BY command_timestamp")
            .fetch_all(pool)
            .await
            .unwrap();

        rows.iter().map(|row| row.get("command")).collect()
    }

    #[tokio::test]
    async fn test_bash_no_date_import()
    {
        let settings = create_test_settings();

        // Import bash history without timestamps
        let result = import_shell_history("_tests/bash-no-date", true, &settings).await;
        assert!(result.is_ok(), "Import should succeed: {:?}", result.err());

        // Verify the data was imported
        let (pool, _) = dejacmd::get_database(&settings.get_local_database_url(), "", "")
            .await
            .unwrap();
        let pool = pool.expect("Pool should exist");

        let count = count_history_entries(&pool).await;
        assert_eq!(count, 4, "Should import 4 commands");

        let commands = get_commands(&pool).await;
        assert!(commands.contains(&"ls -l".to_string()));
        assert!(commands.contains(&"rm -rf /tmp".to_string()));
        assert!(commands.contains(&"fdisk -l".to_string()));
        assert!(commands.contains(&"rsync -avzz /x/ /y/".to_string()));

        cleanup_test_db(&settings);
    }

    #[tokio::test]
    async fn test_bash_with_date_import()
    {
        let settings = create_test_settings();

        // Import bash history with timestamps
        let result = import_shell_history("_tests/bash_date", true, &settings).await;
        assert!(result.is_ok(), "Import should succeed: {:?}", result.err());

        let (pool, _) = dejacmd::get_database(&settings.get_local_database_url(), "", "")
            .await
            .unwrap();
        let pool = pool.expect("Pool should exist");

        let count = count_history_entries(&pool).await;
        assert_eq!(count, 4, "Should import 4 commands");

        let commands = get_commands(&pool).await;
        assert!(commands.contains(&"ls -l".to_string()));
        assert!(commands.contains(&"rm -rf /tmp".to_string()));
        assert!(commands.contains(&"fdisk -l".to_string()));
        assert!(commands.contains(&"cp .zshenv ../me".to_string()));

        // Verify timestamps are correct
        let row = sqlx::query("SELECT command_timestamp FROM history WHERE command = ?")
            .bind("ls -l")
            .fetch_one(&pool)
            .await
            .unwrap();
        let timestamp: String = row.get("command_timestamp");
        // Unix timestamp 1768106005 = 2026-01-11 04:33:25 UTC
        assert!(timestamp.starts_with("2026-01-11"), "Timestamp should be from 2026-01-11, got: {}", timestamp);

        cleanup_test_db(&settings);
    }

    #[tokio::test]
    async fn test_zsh_import()
    {
        let settings = create_test_settings();

        // Import zsh history
        let result = import_shell_history("_tests/zsh", true, &settings).await;
        assert!(result.is_ok(), "Import should succeed: {:?}", result.err());

        let (pool, _) = dejacmd::get_database(&settings.get_local_database_url(), "", "")
            .await
            .unwrap();
        let pool = pool.expect("Pool should exist");

        let count = count_history_entries(&pool).await;
        assert_eq!(count, 6, "Should import 6 commands");

        let commands = get_commands(&pool).await;
        assert!(commands.contains(&"ls -altrh".to_string()));
        assert!(commands.contains(&"env".to_string()));
        assert!(commands.contains(&"cat .zshrc".to_string()));
        assert!(commands.contains(&"sqlite3 .dejacmd.sqlite \"select * from history limit 5\"".to_string()));

        // Verify shell type is zsh
        let row = sqlx::query("SELECT shell FROM history WHERE command = ?")
            .bind("env")
            .fetch_one(&pool)
            .await
            .unwrap();
        let shell: String = row.get("shell");
        assert_eq!(shell, "zsh", "Shell should be zsh");

        cleanup_test_db(&settings);
    }

    #[tokio::test]
    async fn test_mixed_format_import()
    {
        let settings = create_test_settings();

        // Import mixed zsh and bash history
        let result = import_shell_history("_tests/zsh_bash_mix", true, &settings).await;
        assert!(result.is_ok(), "Import should succeed: {:?}", result.err());

        let (pool, _) = dejacmd::get_database(&settings.get_local_database_url(), "", "")
            .await
            .unwrap();
        let pool = pool.expect("Pool should exist");

        let count = count_history_entries(&pool).await;
        assert_eq!(count, 9, "Should import 9 commands");

        let commands = get_commands(&pool).await;

        // Zsh format commands
        assert!(commands.contains(&"ls -altrh".to_string()));
        assert!(commands.contains(&"env".to_string()));
        assert!(commands.contains(&"cat .zshrc".to_string()));

        // Bash with timestamp
        assert!(commands.contains(&"ls -l".to_string()));
        assert!(commands.contains(&"cp .zshenv ../me".to_string()));

        // Bash without timestamp
        assert!(commands.contains(&"ls -ltrh".to_string()));

        // Verify different shell types
        let zsh_count: i64 = sqlx::query("SELECT COUNT(*) as count FROM history WHERE shell = ?")
            .bind("zsh")
            .fetch_one(&pool)
            .await
            .unwrap()
            .get("count");

        let bash_count: i64 = sqlx::query("SELECT COUNT(*) as count FROM history WHERE shell = ?")
            .bind("bash")
            .fetch_one(&pool)
            .await
            .unwrap()
            .get("count");

        assert_eq!(zsh_count, 6, "Should have 6 zsh commands");
        assert_eq!(bash_count, 3, "Should have 3 bash commands");

        cleanup_test_db(&settings);
    }

    #[tokio::test]
    async fn test_truncate_before_import()
    {
        let settings = create_test_settings();

        // First import
        import_shell_history("_tests/bash-no-date", true, &settings).await.unwrap();

        let (pool, _) = dejacmd::get_database(&settings.get_local_database_url(), "", "")
            .await
            .unwrap();
        let pool = pool.expect("Pool should exist");

        let count1 = count_history_entries(&pool).await;
        assert_eq!(count1, 4, "Should have 4 commands after first import");

        // Second import with truncate
        import_shell_history("_tests/zsh", true, &settings).await.unwrap();

        let count2 = count_history_entries(&pool).await;
        assert_eq!(count2, 6, "Should have 6 commands after truncate and second import");

        cleanup_test_db(&settings);
    }

    #[tokio::test]
    async fn test_no_truncate_import()
    {
        let settings = create_test_settings();

        // First import
        import_shell_history("_tests/bash-no-date", false, &settings).await.unwrap();

        let (pool, _) = dejacmd::get_database(&settings.get_local_database_url(), "", "")
            .await
            .unwrap();
        let pool = pool.expect("Pool should exist");

        let count1 = count_history_entries(&pool).await;
        assert_eq!(count1, 4, "Should have 4 commands after first import");

        // Second import without truncate
        import_shell_history("_tests/zsh", false, &settings).await.unwrap();

        let count2 = count_history_entries(&pool).await;
        assert_eq!(count2, 10, "Should have 10 commands total (4 + 6)");

        cleanup_test_db(&settings);
    }

    #[test]
    fn test_parse_zsh_format_valid()
    {
        let line = ": 1768106544:0;ls -altrh";
        let entry = parse_zsh_format(line);

        assert!(entry.is_some());
        let entry = entry.unwrap();
        assert_eq!(entry.timestamp, 1768106544);
        assert_eq!(entry.command, "ls -altrh");
    }

    #[test]
    fn test_parse_zsh_format_with_semicolon_in_command()
    {
        let line = ": 1768106544:0;echo \"test; with semicolon\"";
        let entry = parse_zsh_format(line);

        assert!(entry.is_some());
        let entry = entry.unwrap();
        assert_eq!(entry.timestamp, 1768106544);
        assert_eq!(entry.command, "echo \"test; with semicolon\"");
    }

    #[test]
    fn test_parse_zsh_format_invalid()
    {
        // Missing leading ": "
        assert!(parse_zsh_format("1768106544:0;ls").is_none());

        // Missing semicolon
        assert!(parse_zsh_format(": 1768106544:0 ls").is_none());

        // Invalid timestamp
        assert!(parse_zsh_format(": abc:0;ls").is_none());

        // Empty command
        assert!(parse_zsh_format(": 1768106544:0;").is_some());
    }

    #[tokio::test]
    async fn test_nonexistent_file()
    {
        let settings = create_test_settings();

        let result = import_shell_history("_tests/nonexistent", true, &settings).await;
        assert!(result.is_err(), "Should fail for nonexistent file");
        let err_msg = result.unwrap_err();
        // Error can be either from line counting or from opening the file
        assert!(err_msg.contains("No such file") || err_msg.contains("Failed to open"),
                "Expected file not found error, got: {}", err_msg);

        cleanup_test_db(&settings);
    }

    #[tokio::test]
    async fn test_export_bash_format()
    {
        let settings = create_test_settings();

        // Import test data
        import_shell_history("_tests/bash_date", true, &settings).await.unwrap();

        // Export to bash format
        let export_file = format!("/tmp/test_export_bash_{}.txt", std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos());
        let result = export_shell_history(&export_file, "bash".to_string(), false, &settings).await;
        assert!(result.is_ok(), "Export should succeed: {:?}", result.err());

        // Read and verify the exported file
        let content = std::fs::read_to_string(&export_file).unwrap();
        let lines: Vec<&str> = content.lines().collect();

        // Bash format should have timestamp comments followed by commands
        assert!(lines.len() >= 8, "Should have at least 8 lines (4 commands * 2 lines each)");
        assert!(lines[0].starts_with('#'), "First line should be a timestamp comment");
        assert!(!lines[1].starts_with('#'), "Second line should be a command");

        // Cleanup
        let _ = std::fs::remove_file(&export_file);
        cleanup_test_db(&settings);
    }

    #[tokio::test]
    async fn test_export_zsh_format()
    {
        let settings = create_test_settings();

        // Import test data
        import_shell_history("_tests/zsh", true, &settings).await.unwrap();

        // Export to zsh format
        let export_file = format!("/tmp/test_export_zsh_{}.txt", std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos());
        let result = export_shell_history(&export_file, "zsh".to_string(), false, &settings).await;
        assert!(result.is_ok(), "Export should succeed: {:?}", result.err());

        // Read and verify the exported file
        let content = std::fs::read_to_string(&export_file).unwrap();
        let lines: Vec<&str> = content.lines().collect();

        // Zsh format should have single-line entries starting with ": "
        assert!(lines.len() >= 6, "Should have at least 6 lines");
        for line in &lines {
            assert!(line.starts_with(": "), "Each line should start with ': '");
            assert!(line.contains(";"), "Each line should contain a semicolon separator");
        }

        // Cleanup
        let _ = std::fs::remove_file(&export_file);
        cleanup_test_db(&settings);
    }

    #[tokio::test]
    async fn test_export_and_reimport_bash()
    {
        let settings = create_test_settings();

        // Import original data
        import_shell_history("_tests/bash_date", true, &settings).await.unwrap();

        // Get original count
        let (pool, _) = dejacmd::get_database(&settings.get_local_database_url(), "", "")
            .await
            .unwrap();
        let pool = pool.expect("Pool should exist");
        let original_count = count_history_entries(&pool).await;

        // Export to bash format
        let export_file = format!("/tmp/test_roundtrip_bash_{}.txt", std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos());
        export_shell_history(&export_file, "bash".to_string(), false, &settings).await.unwrap();

        // Re-import the exported file
        import_shell_history(&export_file, true, &settings).await.unwrap();

        // Verify count matches
        let reimported_count = count_history_entries(&pool).await;
        assert_eq!(original_count, reimported_count, "Re-imported count should match original");

        // Cleanup
        let _ = std::fs::remove_file(&export_file);
        cleanup_test_db(&settings);
    }

    #[tokio::test]
    async fn test_export_and_reimport_zsh()
    {
        let settings = create_test_settings();

        // Import original data
        import_shell_history("_tests/zsh", true, &settings).await.unwrap();

        // Get original count
        let (pool, _) = dejacmd::get_database(&settings.get_local_database_url(), "", "")
            .await
            .unwrap();
        let pool = pool.expect("Pool should exist");
        let original_count = count_history_entries(&pool).await;

        // Export to zsh format
        let export_file = format!("/tmp/test_roundtrip_zsh_{}.txt", std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos());
        export_shell_history(&export_file, "zsh".to_string(), false, &settings).await.unwrap();

        // Re-import the exported file
        import_shell_history(&export_file, true, &settings).await.unwrap();

        // Verify count matches
        let reimported_count = count_history_entries(&pool).await;
        assert_eq!(original_count, reimported_count, "Re-imported count should match original");

        // Cleanup
        let _ = std::fs::remove_file(&export_file);
        cleanup_test_db(&settings);
    }

    #[tokio::test]
    async fn test_export_empty_database()
    {
        let settings = create_test_settings();

        // Create an empty database by importing with truncate (this creates the table)
        let (pool, _) = dejacmd::get_database(&settings.get_local_database_url(), "", "")
            .await
            .unwrap();
        if let Some(ref p) = pool {
            sqlx::query(CREATE_TABLE_SQL).execute(p).await.unwrap();
        }

        // Try to export
        let export_file = format!("/tmp/test_export_empty_{}.txt", std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos());
        let result = export_shell_history(&export_file, "bash".to_string(), false, &settings).await;

        // Should succeed but with no entries
        assert!(result.is_ok(), "Export of empty database should succeed: {:?}", result.err());

        // Cleanup (file might not exist if export was skipped)
        let _ = std::fs::remove_file(&export_file);
        cleanup_test_db(&settings);
    }
}
