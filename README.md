# Dejacmd

## Description
Dejacmd is a terminal command history logger that logs commands executed in your terminal to a local and/or central database, allowing you to keep track of your command history across multiple sessions, machines and operating systems. For example, you can log commands to a local SQLite database for quick access and to a central PostgreSQL database for synchronization across devices or you can use the server database (e.g Postgres) to store
commands for multiple users while using an embedded local database (SQLite) for each user,
or just use SQLite locally per user without a central database.

Dejacmd supports databases exposed by the sqlx Rust crate, which are SQLite, PostgreSQL, and MariaDb/MySQL (MSSQL was supported and is currently being redeveloped). It will 
work with Linux (bash, zsh), macOS (bash, zsh) and Windows (PowerShell only) terminals.
(It should also work with fish although it has not been tested yet.)

There are currently two command line programs that combine to provide dejacmd's functionality:

* `dejacmd-log`: This program is intended to be called from your terminal's shell
  configuration file (e.g., `.bashrc`, `.zshrc`, `Microsoft.PowerShell_profile.ps1`) to log each command you execute to database(s).
* `dejacmd`: This program provides a general purpose command line interface to dejacmd
   functionality, such as database url configuration, importing existing history from shell  history files, exporting the database history to shell history files and finally also searching or querying the database for previously executed commands.

A third TUI program may still be developed in the future to provide an interactive interface to search and view command history.

The main dejacmd utility uses a command based interface with subcommands for different functionalities:
```
Usage: dejacmd <COMMAND>

Commands:
  search  
  query   
  config  
  import  
  export  
  help    Print this message or the help of the given subcommand(s)

Options:
  -h, --help  Print help (see more with '--help')

Command Aliases:
search = s or se or sea or sear
query = q or qu or que or quer
config = c or co or con or conf
import = i or im or imp
export = e or ex or exp
```

## Configuration


### Shell Configuration

To log commands executed in your terminal, you need to add functionality to your shell configuration file to call the `dejacmd-log` program.

#### Bash

The old fashioned way is to add the following to your `~/.bashrc` file (or /etc/bash.bashrc for system wide configuration):
```bash
export HISTTIMEFORMAT="%F %T " # note the space at the end
export PROMPT_COMMAND='/usr/local/bin/dejacmd-log -s $? -p $$ "$(history 1)"'
```

A more modern approach which caters for multiple PROMPT_COMMAND uses is to use 
[bash-preexec](https://github.com/rcaloras/bash-preexec) which emulates zsh's preexec and precmd functions:
```bash
#[[ -f ~/.bash-preexec.sh ]] && source ~/.bash-preexec.sh # Default as retrieved from github
[[ -f /usr/share/bash-preexec/bash-preexec.sh ]] && source /usr/share/bash-preexec/bash-preexec.sh # Arch Linux package location for bash-preexec
..
..
dejacmd_hook() {
   HISTTIMEFORMAT="%F %T "
   /usr/local/bin/dejacmd-log -s $? -p $$  "$(history 1)"
}
precmd_functions+=(dejacmd_hook)
```

#### Zsh

Add the following to your `~/.zshrc` file:
```zsh
dejacmd_hook() {
   setopt EXTENDED_HISTORY
   /usr/local/bin/dejacmd-log -s $? -p $$  "$(EXTENDED_HISTORY= fc -t '%Y-%m-%d %T ' -il -1)"
}
precmd_functions+=(dejacmd_hook)
```

#### PowerShell

1. Check if you have a profile script by running: 

```pwsh
Test-Path $PROFILE
```

If it returns false you need to create one by running:

```pwsh
New-Item -Type File -Path $PROFILE -Force
```

2. Open the profile script in a text editor:

```pwsh
notepad $PROFILE
```

3. Add the prompt function to the profile script by adding the following lines:

```pwsh
function prompt {
    # 1. Capture the numerical exit code of the last command
    $lastStatus = $LastExitCode

    # 2. Retrieve the last history item
    $historyItem = Get-History -Count 1

    if ($historyItem) {
        # 3. Replicate HISTTIMEFORMAT="%F %T "
        # %F = yyyy-MM-dd, %T = HH:mm:ss
        $timestamp = $historyItem.StartExecutionTime.ToString("yyyy-MM-dd HH:mm:ss")

        # 4. Construct the history string: "ID  TIMESTAMP  COMMAND"
        $historyString = "$($historyItem.Id)  $timestamp $($historyItem.CommandLine)"

        # 5. Invoke dejacmd program *using $HOME to represent the user home directory)
        $loggerPath = "$HOME/bin/dejacmd-log"
        if (Test-Path $loggerPath) {
            & $loggerPath -s $lastStatus $historyString
        }
    }
    # Standard prompt return
    "PS $($executionContext.SessionState.Path.CurrentLocation)> "
}
```

4. Save and logout/exit to restart PowerShell.

### Database Configuration

Dejacmd uses a JSON configuration file located at:

* Linux: `~/.config/dejacmd/settings.json`
* Macos: `~/.config/dejacmd/settings.json` or `~/Library/Application Support/dejacmd/settings.json` or `~/dejacmd/settings.json` 
* Windows: `%APPDATA%\dejacmd\settings.json` (`AppData/Local/dejacmd/settings.json` or `Local Settings/dejacmd/settings.json` or `Application Data/dejacmd/Local Settings/settings.json`) 

The dejacmd program allows for database configuration through the config subcommand:
```
dejacmd config --help
Usage: dejacmd config [OPTIONS]
Options:
  -L, --local-database [<LOCAL_URL>]
          Get or set local database URL in settings file [default sqlite://~/.dejacmd.sqlite].
                      When setting {{user}} and {{password}} can be used as placeholders for username and password respectively (use -u and -p options for user and password).
                      Password will be encrypted in the settings file.
                      Use ~ for the user home directory if using SQLite which will be fully expanded when written.
                      Examples: dejacmd config -L "sqlite://~/Documents/dejacmd.sqlite
                      dejacmd config -L "postgresql://{{user}}:{{password}}@localhost/myowndb" -u postgres -p"pAssword" 
  -C, --central-database [<CENTRAL_URL>]
          Get or set Central database URL in settings file.
                      When setting {{user}} and {{password}} can be used as placeholders for username and password respectively (use -u and -p options for user and password).
                      Password will be encrypted in the settings file.
                      Use ~ for the user home directory if using SQLite which will be fully expanded when written.
                      Examples: dejacmd config -C "postgresql://{{user}}:{{password}}@localhost/dejacmd" -u postgres -p
                      dejacmd config -C "mysql://{{user}}:{{password}}@localhost/dejacmd" -u me -p
                      dejacmd config -C "sqlite:///home/share/history/dejacmd.sqlite
  -u, --user <USER>
          Database user to use with -L or -C database URLs for databases where authentication is required. [default: ]
  -p, --password [<PASSWORD>]
          Database password to use with -L or -C database URLs for databases where authentication is required..
                      If flag is present but no value provided, will prompt for password
  -s, --show
          Show password when entering from console
  -h, --help
          Print help
```


## Import/Export History
You can import existing shell history into the dejacmd database using the `dejacmd import`:
```
dejacmd import --help 
Arguments:
  <SHELL_HISTORY_FILE>  Shell history file e.g .bash_history or recent SQLite database e.g ~/.recent.db

Options:
  -T, --truncate  Truncate history table before importing
  -h, --help      Print help
Example:
  dejacmd import ~/.bash_history
  dejacmd import -T ~/.zsh_history  
```

The truncate option allows you to clear the existing history in the database before importing. The import handles various flavors of both bash and zsh history files, 
and even files containing mixtures of both bash and zsh history entries.

In the above recent refers to earlier python based related projects [recent](https://github.com/trengrj/recent) and [recent2](https://github.com/dotslash/recent2/) which logged commands to a local SQLite database named `recent.db` in the user home directory. Dejacmd can import history from these databases as well. 

Exporting history from the dejacmd database to a shell history file is done using the `dejacmd export` command:
```
Usage: dejacmd export [OPTIONS] <EXPORT_HISTORY_FILE>
Arguments:
  <EXPORT_HISTORY_FILE>  Export to a bash or zsh history file
Options:
  -E, --format <EXPORT_HISTORY_FORMAT>  Export format: bash or zsh [bash] [default: bash]
  -F, --from-central                    Export history from central database if configured (defaults to local database)
  -h, --help                            Print help
Example:
  dejacmd export ~/.bash_history  
```

### Searching and Querying History
You can search the dejacmd database for previously executed commands using the `dejacmd search` command:
```
sage: dejacmd search [OPTIONS] [SEARCH_SPEC]

Arguments:
  [SEARCH_SPEC]  Command line history search string filter

Options:
      --central             Search central database if configured (defaults to local database).
  -n, --lines <NUMBER>      Number of lines to show from history [default: 25]
  -i, --no-case             Case insensitive search
  -t, --no-time             Don't show timestamps in output
  -u, --unique              Filter out duplicate commands in output (implies -t no timestamps)
  -s, --start <START_TIME>  Start timestamp for search in YYYY-MM-DD_HH:MM:SS format (if no time specified, assumes 00:00:00) [default: ]
  -e, --end <END_TIME>      End timestamp for search in YYYY-MM-DD_HH:MM:SS format (if no time specified, assumes 00:00:00) [default: ]
  -h, --help                Print help

Examples:
   dejacmd search "rsync -avz" -n 10
   dejacmd s -u "ls -al"
   dejacmd se  "df -h" -s 2024-03-01_13:00:00 -e 2024-03-31_13:00:00
```

### Querying the Database Directly
For more advanced searches, you can use the `dejacmd query` command to execute raw SQL queries against the database:
```
dejacmd query [OPTIONS] [SQL]

Arguments:
  [SQL]  Custom SQL query to execute against history database

Options:
      --central  Query central database if configured (defaults to local database).
  -D, --ddl      Show the DDL for the history table (for custom queries)
  -h, --help     Print help

Examples:
   dejacmd query "SELECT command, command_timestamp FROM history WHERE shell='bash' LIMIT 10"
   dejacmd query "SELECT DISTINCT shell FROM history"
   dejacmd query "SELECT COUNT(*) FROM history WHERE command LIKE '%docker%'"
   dejacmd query --central "SELECT * FROM history ORDER BY command_timestamp DESC LIMIT 5"

Note: If no query is provided, you will be prompted to enter one interactively.
```

You can also use any other SQL client to query the databases directly if you prefer.

## Related Projects
As noted in the import/export section, the concept is based on earlier projects named  [recent](https://github.com/trengrj/recent) and [recent2](https://github.com/dotslash/recent2/) which logged commands to a local SQLite database named `.recent.db` in the user home directory. Dejacmd extends this functionality to support multiple database backends, central databases, and more advanced querying and configuration options, and does not depend on Python i.e the entire Python runtime does not need to be loaded into memory for every command line invocation.
