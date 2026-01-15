use std::error::Error;

use colored::Colorize;
use sqlx::{Any, AnyPool, Pool};

use crate::settings::Settings;

pub mod crypt;
pub mod settings;

pub const CREATE_TABLE_SQL: &str = 
r#" CREATE TABLE IF NOT EXISTS history 
(
   id VARCHAR(255) PRIMARY KEY,
   command_timestamp TEXT NOT NULL,
   cwd TEXT,
   shell TEXT,
   user_id BIGINT,
   user_name TEXT,
   ip TEXT,
   os TEXT,
   exit_status BIGINT,
   command TEXT
)"#;

pub const CREATE_INDEX_SQL: &str = 
r#" CREATE INDEX IF NOT EXISTS idx_history_timestamp ON history (command_timestamp);
"#;

pub const INSERT_HISTORY_SQL: &str = 
r#"INSERT INTO history (id, command_timestamp, cwd, shell, user_id, user_name, ip, os, exit_status, command) 
VALUES ( ?, ?, ?, ?, ?, ?, ?, ?, ?, ? )"#;


pub async fn get_database(url: &str, user: &str, password: &str) -> Result<(Option< Pool<Any> >, String), Box<dyn Error>>
//---------------------------------------------------------------------------------
{
   // Handle empty URL - return None pool
   if url.trim().is_empty()
   {
      return Ok((None, String::new()));
   }

   let mut database_url = url.to_string();
   let mut error_url = database_url.clone();
   let scheme = database_url.split("://").next().unwrap_or("").to_string();
   if scheme.starts_with("postgres") || scheme.starts_with("mysql") || scheme.starts_with("mssql")
   {      
      if user.is_empty() && password.is_empty()
      {
         let p = database_url.find("@");
         if let Some(pos) = p
         {
            database_url = format!("{}{}", scheme, &database_url[(pos + 1)..]);
            error_url = database_url.clone();
         }
      }
      else
      {  
         let dburl = database_url.replace("{{user}}", user);
         let err_url = error_url.replace("{{user}}", user);
         let n = password.len();
         database_url = dburl.replace("{{password}}", password);
         error_url = err_url.replace("{{password}}", "*".repeat(n).as_str());
      }   
   }
   else if scheme.starts_with("sqlite")
   {
      // On Windows, check if we have an absolute path and need an extra slash
      #[cfg(target_os = "windows")]
      {
         // Check if this is an absolute Windows path (e.g., sqlite://C:\ or sqlite://D:\)
         // The format should be sqlite:///C:\ (three slashes total)
         if let Some(path_part) = database_url.strip_prefix("sqlite://")
         {
            // Check if it looks like a Windows drive letter path (e.g., C:\, D:\)
            if path_part.len() >= 2 && path_part.chars().nth(1) == Some(':')
            {
               // Add the extra slash to make it sqlite:///
               database_url = format!("sqlite:///{}", path_part);
               error_url = database_url.clone();
            }
         }
      }

      // Also don't add mode=rwc to in-memory databases in case used in for tests.
      if ! database_url.contains("mode=rwc") && ! database_url.contains(":memory:")
      {
         if database_url.contains("?")
         {
            database_url = format!("{}&mode=rwc", database_url);
         }
         else
         {
            database_url = format!("{}?mode=rwc", database_url);
         }
         error_url = database_url.clone();
      }
   }
   else
   {
      return Err( Box::new( std::io::Error::other(
         format!("{} {} [{}]", "Unsupported database scheme: ".red(), scheme.red(), "Supported schemes are: sqlite, postgres, mysql, mssql".bright_red()) ) ) );
   }
   let pool = match AnyPool::connect(&database_url).await
   {
      Ok(p) => p,
      Err(e) =>
      {
         return Err( Box::new( std::io::Error::other(
            format!("{} {} [{}]", "Error connecting to database: ".red(), error_url.red(), e.to_string().bright_red()) ) ) );
      }
   };   
   Ok((Some(pool), scheme))
}

pub fn fix_placeholders(sql: &str, scheme: &str) -> String
//--------------------------------------------------------------
{
    if scheme.starts_with("postgres") //|| scheme.starts_with("sqlite") sqlite seems to work both ways
    {
        let mut n = sql.matches("?").count();
        let mut c = 1;
        let mut s= sql.to_string();
        while n > 0
        {
            let rep = format!("${}", c);
            s = s.replacen("?", &rep, 1);
            c += 1;
            n = s.matches("?").count();
        }
        s
    }
    else
    {
        sql.to_string()
    }
}

pub async fn connections(settings: &Settings, is_create: bool, is_truncate: bool) ->
   Result<(Option<sqlx::Pool<sqlx::Any>>, String, Option<sqlx::Pool<sqlx::Any>>, String), String>
//----------------------------------------------------------------------------------------------------------------------------------------
{
   // Connect to database
   let local_url = settings.get_local_database_url();
   let central_url = settings.get_central_database_url();

   let (local_user, local_password) = match settings.get_credentials(true)
   {
      Ok((u, p)) => (u, p),
      Err(_) => ("".to_string(), "".to_string())
   };

   let (local_pool_opt, local_scheme) = match get_database(&local_url, &local_user, &local_password).await
   {
      Ok((p, s)) => (p, s),
      Err(e) => return Err(format!("Error connecting to database: {}", e)),
   };

   let (central_user, central_password) = match settings.get_credentials(false)
   {
      Ok((u, p)) => (u, p),
      Err(_) => ("".to_string(), "".to_string())
   };

   let (central_pool_opt, central_scheme) = match get_database(&central_url, &central_user, &central_password).await
   {
      Ok((p, s)) => (p, s),
      Err(e) => return Err(format!("Error connecting to database: {}", e)),
   };
   if is_create
   {
      if let Some(ref local_pool) = local_pool_opt
      {
         sqlx::query(CREATE_TABLE_SQL).execute(local_pool).await
         .map_err(|e| format!("Error creating table: {}", e))?;
      };

      if let Some(ref central_pool) = central_pool_opt
      {
         sqlx::query(CREATE_TABLE_SQL).execute(central_pool).await
         .map_err(|e| format!("Error creating table: {}", e))?;
      };
   }
   if is_truncate
   {
      if let Some(ref local_pool) = local_pool_opt
      {
         sqlx::query("DELETE FROM history").execute(local_pool).await
         .map_err(|e| format!("Error truncating local history table: {}", e))?;
      };

      if let Some(ref central_pool) = central_pool_opt
      {
         sqlx::query("DELETE FROM history").execute(central_pool).await
         .map_err(|e| format!("Error truncating central history table: {}", e))?;
      };
   }
   Ok((local_pool_opt, local_scheme, central_pool_opt, central_scheme))
}
