# Opsfolio Resource Surveillance

`surveilr` is an extendable file system inspector for performing
surveillance of machine resources. It is used to walk resources like file
systems and generate an SQLite database which can then be consumed by any
computing environment that supports SQLite.

## Installation

You can install the latest `surveilr` using either of the following one-liners:

```bash
$ curl -sL https://raw.githubusercontent.com/opsfolio/resource-surveillance/main/install.sh | sh

# if you want a different install path
$ SURVEILR_HOME="$HOME/bin" curl -sL https://raw.githubusercontent.com/opsfolio/resource-surveillance/main/install.sh | sh
```

If you use `eget`:

```bash
$ eget opsfolio/resource-surveillance --asset tar.gz
```

## Usage

```bash
$ ./surveilr --help                         # get CLI help
$ ./surveilr fs-walk                        # walk the current working directory (CWD)
$ ./surveilr fs-walk -r /other -r /other2   # walk some other director(ies)
$ ./surveilr fs-walk --stats                # walk the current working directory (CWD) show stats afterwards
```

Other use cases:

```bash
$ ./surveilr notebooks ls                                     # list all notebooks and cells available, with migrations status
$ ./surveilr notebooks cat --cell infoSchemaOsQueryATCs       # export the information schema as osQuery ATC
$ ./surveilr notebooks cat --cell notebooksInfoSchemaDiagram  # show the notebooks admin PlanUML ERD stored in the database
$ ./surveilr notebooks cat --cell surveilrInfoSchemaDiagram   # show the surveilr PlanUML ERD stored in the database
$ ./surveilr --completions fish | source                      # setup completions to reduce typing
```

In order to ensure that the Resource Surveillance agent is extensible, we leverage SQLite heavily for both storage of data but also storing the SQL it needs to bootstrap itself, perform migrations, and conduct regular administrative and query operations.

The key to that extensibility is the `code_notebook_cell` table which stores SQL (called _SQL notebook cells_) or other interpretable code in the database so that once the database is created, all SQL and related code is part of the database and may be executed like this from the CLI using any environment that supports SQLite shell or SQLite drivers:

```bash
$ sqlite3 xyz.db "select sql from code_notebook_cell where code_notebook_cell_id = 'infoSchemaMarkdown'" | sqlite3 xyz.db
```

You can pass in arguments using .parameter or `sql_parameters` table, like:
```bash
$ echo ".parameter set X Y; $(sqlite3 xyz.db \"SELECT sql FROM code_notebook_cell where code_notebook_cell_id = 'init'\")" | sqlite3 xyz.db
```

Anywhere you see `./surveilr notebooks cat` those can be run directly through SQLite, the following two commands do the same thing:

```bash
$ ./surveilr notebooks cat --cell infoSchemaOsQueryATCs | sqlite3 resource-surveillance.sqlite.db
$ sqlite3 resource-surveillance.sqlite.db "select interpretable_code from stored_notebook_cell where cell_name = 'infoSchemaOsQueryATCs'" | sqlite3 device-content.sqlite.db
```

See [CLI Help](CLI-help.md) for details.

## Architecture

![Architecture](architecture.drawio.svg)

Device Uniform Resource ER Diagram (generated from
`./surveilr notebooks cat --cell surveilrInfoSchemaDiagram`) without
housekeeping columns:

![Uniform Resource ER Diagram](device-ur-er-diagram.png)

Administrative Code Notebooks ER Diagram (generated from
`./surveilr notebooks cat --cell notebooksInfoSchemaDiagram`):

![Notebooks ER Diagram](notebooks-er-diagram.png)

## Development

**IMPORTANT**: Use SQLa to generate all SQL so it's portable but use Rusqlite to
make working with SQLite more ergonomic. Remember to only use libraries to help
improve developer productivity, always assume SQLite database will be used
across polyglot programming environments so SQL code should be transparent and
portable.

Development prerequisites:

- Install Rust toolchain (1.73 or above, best to use `rustup`, `asdf` or `rtx`
  for multiple simultaneous versions)
- `cargo install just` so we can use `Justfile` for task management

Regular use:

```bash
$ just --completions fish | source            # setup completions to reduce typing

$ just test                                   # run unit tests with cargo nextest

$ just run                                    # get CLI help
$ cargo run -- --help                         # get CLI help, same as above

$ just run admin cli-help-md                  # get CLI in Markdown and update this README.md manually
$ cargo run -- --help-markdown > CLI-help.md  # get CLI in Markdown, same as above

$ just run fs-walk --help                     # get CLI help for fs-walk subcommand
$ just run --debug fs-walk                    # walk the current working directory (CWD) with debug messages
$ just run fs-walk -r /other -r /other2       # walk some other director(ies)
$ just run fs-walk -i .git/                   # walk CWD, ignore .git/ paths
$ just run fs-walk -i .git/ -i target/        # walk CWD, ignore .git/ and target/ paths

$ just sqla-sync                              # generate SQLa bootstrap and other SQL
                                             
$ just dev                                    # turn on auto-compile, auto-run during development
                                              # using cargo-watch command
```

## ULID Primary Keys across multiple devices

The ULID (Universally Unique Lexicographically Sortable Identifier) is designed
to generate unique identifiers across distributed systems without coordination.
Let's break down its structure to understand the likelihood of conflicts.

A ULID consists of:

1. A 48-bit timestamp, which gives millisecond precision for about 138 years.
2. A 80-bit random component, which is generated randomly for each ID.

Given the design, there are two primary scenarios where a conflict might arise:

1. **Timestamp Collision**: If two or more ULIDs are generated at the exact same
   millisecond on different machines, then the uniqueness of the ULID is purely
   dependent on the randomness of the second component.
2. **Randomness Collision**: Even if the timestamp differs, if the random
   component generated is the same for two ULIDs (which is astronomically
   unlikely), there will be a conflict.

Now, let's consider the probability of each scenario:

1. **Timestamp Collision**: If you're generating millions of ULIDs in a short
   span of time, it's quite likely that you'll have multiple ULIDs with the same
   timestamp. This isn't a problem by itself, but it means the uniqueness then
   rests on the random component.
2. **Randomness Collision**: The random component of a ULID is 80 bits. This
   means there are \(2^{80}\) or approximately \(1.2 x 10^{24}\) possible
   values. If you generate millions (let's say one million for simplicity), the
   chance of any two having the same random value is tiny. Using the birthday
   paradox as a rough estimation, even after generating billions of ULIDs, the
   probability of a conflict in the random component remains astronomically low.

If you generate millions of ULIDs across multiple machines, the chances of a
collision are extremely low due to the large randomness space of the 80-bit
random component. The design of the ULID ensures that even in high-throughput
distributed systems, conflicts should be virtually non-existent. As always with
such systems, monitoring and conflict resolution strategies are still good
practices, but the inherent risks are minimal.
