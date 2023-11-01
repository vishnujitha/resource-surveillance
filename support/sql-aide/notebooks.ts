import {
  chainNB,
  cmdNB,
  frontmatter as fm,
  SQLa,
  SQLa_tp as typical,
  sysinfo as si,
  ulid,
} from "./deps.ts";
import * as m from "./models.ts";

// deno-lint-ignore no-explicit-any
type Any = any;

// TODO: Integrate SQLa Quality System functionality so that documentation is not just in code but
// makes its way into the database.

// TODO: move this to README.md:
// ServiceContentHelpers creates "insertable" type-safe content objects needed by the other notebooks
// SqlNotebookHelpers encapsulates instances of SQLa objects needed to creation of SQL text in the other notebooks
// ConstructionSqlNotebook encapsulates DDL and table/view/entity construction
// MutationSqlNotebook encapsulates DML and stateful table data insert/update/delete
// QuerySqlNotebook encapsulates DQL and stateless table queries that can operate all within SQLite
// AssuranceSqlNotebook encapsulates DQL and stateless TAP-formatted test cases
// PolyglotSqlNotebook encapsulates SQL needed as part of multi-language (e.g. SQLite + Deno) orchestration because SQLite could not operate on its own
// SQLPageNotebook encapsulates [SQLPage](https://sql.ophir.dev/) content
// SqlNotebooksOrchestrator encapsulates instances of all the other notebooks and provides performs all the work

// Reminders:
// - when sending arbitrary text to the SQL stream, use SqlTextBehaviorSupplier
// - when sending SQL statements (which need to be ; terminated) use SqlTextSupplier
// - use jtladeiras.vscode-inline-sql, frigus02.vscode-sql-tagged-template-literals-syntax-only or similar SQL syntax highlighters in VS Code so it's easier to edit SQL

/**
 * MORE TODO for README.md:
 * Our SQL "notebook" is a library function which is responsible to pulling
 * together all SQL we use. It's important to note we do not prefer to use ORMs
 * that hide SQL and instead use stateless SQL generators like SQLa to produce
 * all SQL through type-safe TypeScript functions.
 *
 * Because applications come and go but data lives forever, we want to allow
 * our generated SQL to be hand-edited later if the initial generated code no
 * longers benefits from being regenerated in the future.
 *
 * We go to great lengths to allow SQL to be independently executed because we
 * don't always know the final use cases and we try to use the SQLite CLI whenever
 * possible because performance is best that way.
 *
 * Because SQL is a declarative and TypeScript is imperative langauage, use each
 * for their respective strengths. Use TypeScript to generate type-safe SQL and
 * let the database do as much work as well.
 * - Capture all state, valid content, invalid content, and other data in the
 *   database so that we can run queries for observability; if everything is in
 *   the database, including error messages, warnings, etc. we can always run
 *   queries and not have to store logs in separate system.
 * - Instead of imperatively creating thousands of SQL statements, let the SQL
 *   engine use CTEs and other capabilities to do as much declarative work in
 *   the engine as possible.
 * - Instead of copy/pasting SQL into multiple SQL statements, modularize the
 *   SQL in TypeScript functions and build statements using template literal
 *   strings (`xyz${abc}`).
 * - Wrap SQL into TypeScript as much as possible so that SQL statements can be
 *   pulled in from URLs.
 * - If we're importing JSON, CSV, or other files pull them in via
 *   `import .. from "xyz" with { type: "json" }` and similar imports in case
 *   the SQL engine cannot do the imports directly from URLs (e.g. DuckDB can
 *   import HTTP directly and should do so, SQLite can pull from URLs too with
 *   the http0 extension).
 * - Whenever possible make SQL stateful functions like DDL, DML, etc. idempotent
 *   either by using `ON CONFLICT DO NOTHING` or when a conflict occurs put the
 *   errors or warnings into a table that the application should query.
 */

/**
 * Decorate a function with `@notIdempotent` if it's important to indicate
 * whether its SQL is idempotent or not. By default we assume all SQL is
 * idempotent but this can be set to indicate it's not.
 */
export const notIdempotent = <Notebook>(
  cells: Set<chainNB.NotebookCellID<Notebook>>,
) => {
  return (
    _target: SQLa.SqlNotebook<Any>,
    propertyKey: chainNB.NotebookCellID<Notebook>,
    _descriptor: PropertyDescriptor,
  ) => {
    cells.add(propertyKey);
  };
};

/**
 * Decorate a function with `@dontStoreInDB` if the particular query should
 * not be stored in the code_notebook_cell table in the database.
 */
export const dontStoreInDB = <Notebook>(
  cells: Set<chainNB.NotebookCellID<Notebook>>,
) => {
  return (
    _target: SQLa.SqlNotebook<Any>,
    propertyKey: chainNB.NotebookCellID<Notebook>,
    _descriptor: PropertyDescriptor,
  ) => {
    cells.add(propertyKey);
  };
};

export const noSqliteExtnLoader: (
  extn: string,
) => SQLa.SqlTextBehaviorSupplier<Any> = (extn: string) => ({
  executeSqlBehavior: () => ({
    SQL: () => `-- loadExtnSQL not provided to load '${extn}'`,
  }),
});

async function gitLikeHash(content: string) {
  // Git header for a blob object (change 'blob' to 'commit' or 'tree' for those objects)
  // This assumes the content is plain text, so we can get its length as a string
  const header = `blob ${content.length}\0`;

  // Combine header and content
  const combinedContent = new TextEncoder().encode(header + content);

  // Compute SHA-1 hash
  const hashBuffer = await crypto.subtle.digest("SHA-1", combinedContent);

  // Convert hash to hexadecimal string
  const hashArray = Array.from(new Uint8Array(hashBuffer));
  const hashHex = hashArray.map((b) => b.toString(16).padStart(2, "0")).join(
    "",
  );

  return hashHex;
}

/**
 * Creates "insertable" type-safe content objects needed by the other notebooks
 * (especially for DML/mutation SQL).
 */
export class ServiceContentHelpers<
  EmitContext extends SQLa.SqlEmitContext = SQLa.SqlEmitContext,
> {
  constructor(
    readonly models: ReturnType<typeof m.serviceModels<EmitContext>>,
  ) { }

  activeDevice(boundary?: string) {
    return {
      name: Deno.hostname(),
      boundary: boundary ?? "??",
    };
  }

  async activeDeviceInsertable(deviceId = { SQL: () => "ulid()" }) {
    const ad = this.activeDevice();
    return this.models.device.prepareInsertable({
      deviceId,
      name: ad.name,
      boundary: ad.boundary,
      deviceElaboration: JSON.stringify({
        hostname: Deno.hostname(),
        networkInterfaces: Deno.networkInterfaces(),
        osPlatformName: si.osPlatformName(),
        osArchitecture: await si.osArchitecture(),
      }),
    });
  }
}

/**
 * Encapsulates instances of SQLa objects needed to creation of SQL text in the
 * other notebooks. An instance of this class is usually passed into all the
 * other notebooks.
 */
export class SqlNotebookHelpers<
  EmitContext extends SQLa.SqlEmitContext = SQLa.SqlEmitContext,
> extends SQLa.SqlNotebook<EmitContext> {
  readonly emitCtx: EmitContext;
  readonly sch: ServiceContentHelpers<EmitContext>;
  readonly models: ReturnType<typeof m.serviceModels<EmitContext>>;
  readonly loadExtnSQL: (
    extn: string,
  ) => SQLa.SqlTextBehaviorSupplier<EmitContext>;
  readonly stsOptions: SQLa.SqlTextSupplierOptions<EmitContext>;
  readonly modelsGovn: ReturnType<
    typeof m.serviceModels<EmitContext>
  >["codeNbModels"]["modelsGovn"];
  readonly templateState: ReturnType<
    typeof m.serviceModels<EmitContext>
  >["codeNbModels"]["modelsGovn"]["templateState"];

  constructor(
    readonly options?: {
      readonly loadExtnSQL?: (
        extn: string,
      ) => SQLa.SqlTextBehaviorSupplier<EmitContext>;
      readonly models?: ReturnType<typeof m.serviceModels<EmitContext>>;
      readonly stsOptions?: SQLa.SqlTextSupplierOptions<EmitContext>;
    },
  ) {
    super();
    this.models = options?.models ?? m.serviceModels<EmitContext>();
    this.modelsGovn = this.models.codeNbModels.modelsGovn;
    this.emitCtx = this.modelsGovn.sqlEmitContext();
    this.sch = new ServiceContentHelpers(this.models);
    this.templateState = this.modelsGovn.templateState;
    this.loadExtnSQL = options?.loadExtnSQL ?? noSqliteExtnLoader;
    this.stsOptions = options?.stsOptions ??
      SQLa.typicalSqlTextSupplierOptions();
  }

  // type-safe wrapper for all SQL text generated in this library;
  // we call it `SQL` so that VS code extensions like frigus02.vscode-sql-tagged-template-literals
  // properly syntax-highlight code inside SQL`xyz` strings.
  get SQL() {
    return SQLa.SQL<EmitContext>(this.templateState.ddlOptions);
  }

  renderSqlCmd() {
    return SQLa.RenderSqlCommand.renderSQL<EmitContext>((sts) =>
      sts.SQL(this.emitCtx)
    );
  }

  // type-safe wrapper for all SQL that should not be treated as SQL statements
  // but as arbitrary text to send to the SQL stream
  sqlBehavior(
    sts: SQLa.SqlTextSupplier<EmitContext>,
  ): SQLa.SqlTextBehaviorSupplier<EmitContext> {
    return {
      executeSqlBehavior: () => sts,
    };
  }

  // ULID generator when the value is needed in JS runtime
  get newUlid() {
    return ulid.ulid;
  }

  // ULID generator when the value is needed by the SQLite engine runtime
  get sqlEngineNewUlid(): SQLa.SqlTextSupplier<EmitContext> {
    return { SQL: () => `ulid()` };
  }

  get onConflictDoNothing(): SQLa.SqlTextSupplier<EmitContext> {
    return { SQL: () => `ON CONFLICT DO NOTHING` };
  }

  // ULID generator when the value is needed by the SQLite engine runtime
  get sqlEngineNow(): SQLa.SqlTextSupplier<EmitContext> {
    return { SQL: () => `CURRENT_TIMESTAMP` };
  }

  // See [SQLite Pragma Cheatsheet for Performance and Consistency](https://cj.rs/blog/sqlite-pragma-cheatsheet-for-performance-and-consistency/)
  get optimalOpenDB() {
    return this.sqlBehavior(this.SQL`
      -- make sure all pragmas are silent in case SQL will be piped
      .output /dev/null
      PRAGMA journal_mode = wal; -- different implementation of the atomicity properties
      PRAGMA synchronous = normal; -- synchronise less often to the filesystem
      PRAGMA foreign_keys = on; -- check foreign key reference, slightly worst performance
      .output stdout`);
  }

  get optimalCloseDB() {
    return this.sqlBehavior(this.SQL`
      -- make sure all pragmas are silent in case SQL will be piped
      .output /dev/null
      PRAGMA analysis_limit=400; -- make sure pragma optimize does not take too long
      PRAGMA optimize; -- gather statistics to improve query optimization
      -- we do not need .output stdout since it's the last statement of the stream`);
  }

  /**
   * Setup the SQL bind parameters; object property values will be available as
   * :key1, :key2, etc.
   * @param shape is an object with key value pairs that we want to convert to SQLite parameters
   * @returns the rewritten object (using new keys) and the associated DML
   */
  sqlParameters<
    Shape extends Record<
      string,
      string | number | SQLa.SqlTextSupplier<EmitContext>
    >,
  >(shape: Shape) {
    /**
     * This is a "virtual" table that should not be used for DDL but used for DML.
     * It is managed by SQLite and is used to store `.parameter set` values and
     * allows all keys to be used as `:xyz` variables that point to `value`.
     *
     * SQLite shell `.parameter set xyz value` is equivalent to `INSERT INTO
     * sqlite_parameters (key, value) VALUES ('xyz', 'value')` but `.parameter set`
     * does not support SQL expressions. If you need a value to be evaluated before
     * being set then use `INSERT INTO sqlite_parameters (key, value)...`.
     */
    const { model: gm, domains: gd } = this.modelsGovn;
    const sqp = gm.table("sqlite_parameters", {
      key: gd.text(),
      value: gd.text(),
    });

    const paramsDML = Object.entries(shape).map(([key, value]) =>
      sqp.insertDML({
        key: `:${key}`,
        value: typeof value === "number" ? String(value) : value,
      })
    );

    type SqlParameters = { [K in keyof Shape as `:${string & K}`]: Shape[K] };
    return {
      params: (): SqlParameters => {
        const newShape: Partial<SqlParameters> = {};
        for (const key in shape) {
          const newKey = `:${key}`;
          (newShape as Any)[newKey] = shape[key];
        }
        return newShape as unknown as SqlParameters;
      },
      paramsDML,
    };
  }

  viewDefn<ViewName extends string, DomainQS extends SQLa.SqlDomainQS>(
    viewName: ViewName,
  ) {
    return SQLa.viewDefinition<ViewName, EmitContext, DomainQS>(viewName, {
      isIdempotent: true,
      embeddedStsOptions: this.templateState.ddlOptions,
      before: (viewName) => SQLa.dropView(viewName),
    });
  }

  /**
   * Handler which can be passed into Command Notebook pipe options.stdIn to
   * automatically wrap the spawn results into database rows for convenient
   * access to strongly typed data from previous command's execution result.
   * @param handler function which accepts spawned content and writes to STDIN for next command
   * @returns a handler function to pass into pipe options.stdIn
   */
  pipeInSpawnedRows<Row>(
    handler: (
      rows: Row[],
      write: (text: string | SQLa.SqlTextSupplier<EmitContext>) => void,
      nbh: SqlNotebookHelpers<EmitContext>,
    ) => Promise<void>,
  ): cmdNB.SpawnedResultPipeInWriter {
    const te = new TextEncoder();
    const td = new TextDecoder();
    return async (sr, rawWriter) => {
      const writer = (text: string | SQLa.SqlTextSupplier<EmitContext>) => {
        if (typeof text === "string") {
          rawWriter.write(te.encode(text));
        } else {
          rawWriter.write(te.encode(text.SQL(this.emitCtx)));
        }
      };
      await handler(JSON.parse(td.decode(sr.stdout)) as Row[], writer, this);
    };
  }
}

/**
 * Encapsulates SQL DDL and table/view/entity construction SQLa objects. The
 * actual models are not managed by this class but it does include all the
 * migration scripts which assemble the other SQL into a migration steps.
 *
 * TODO: add ISML-like migration capabilities so that database evoluation is
 * managed automatically. `existingMigrations` in the constructor is the list
 * of migrations already performed so this system needs to be smart enough to
 * generate SQL only for migrations that have not yet been performed.
 */
export class ConstructionSqlNotebook<
  EmitContext extends SQLa.SqlEmitContext = SQLa.SqlEmitContext,
> extends SQLa.SqlNotebook<EmitContext> {
  static readonly notIdempodentCells = new Set<keyof ConstructionSqlNotebook>();

  constructor(
    readonly nbh: SqlNotebookHelpers<EmitContext>,
    readonly storedNotebookStateTransitions: ReturnType<
      typeof nbh.models.codeNbModels.codeNotebookState.select
    >["filterable"][],
  ) {
    super();
  }

  @notIdempotent(ConstructionSqlNotebook.notIdempodentCells)
  bootstrapDDL() {
    const { nbh, nbh: { models: { codeNbModels } } } = this;
    // deno-fmt-ignore
    return nbh.SQL`
      ${codeNbModels.informationSchema.tables}

      ${codeNbModels.informationSchema.tableIndexes}
      `;
  }

  bootstrapSeedDML() {
    const { nbh, nbh: { models: { codeNbModels: { codeNotebookKernel: kernel } } } } = this;
    const created_at = nbh.sqlEngineNow;
    const sql = kernel.insertDML({
      code_notebook_kernel_id: "SQL",
      kernel_name: "Dialect-independent ANSI SQL",
      mime_type: "application/sql",
      file_extn: ".sql",
      created_at
    });
    const puml = kernel.insertDML({
      code_notebook_kernel_id: "PlantUML",
      kernel_name: "PlantUML ER Diagram",
      mime_type: "text/vnd.plantuml",
      file_extn: ".puml",
      created_at
    });
    return [sql, puml];
  }

  @notIdempotent(ConstructionSqlNotebook.notIdempodentCells)
  initialDDL() {
    const { nbh, nbh: { models } } = this;
    // deno-fmt-ignore
    return nbh.SQL`
      ${models.informationSchema.tables}

      ${models.informationSchema.tableIndexes}
      `;
  }

  fsContentWalkSessionStatsViewDDL() {
    // deno-fmt-ignore
    return this.nbh.viewDefn("fs_content_walk_session_stats")/* sql */`
      WITH Summary AS (
          SELECT
              strftime('%Y-%m-%d %H:%M:%S.%f', fcws.walk_started_at) AS walk_datetime,
              strftime('%f', fcws.walk_finished_at - fcws.walk_started_at) AS walk_duration,
              COALESCE(fcwpe.file_extn, '') AS file_extn,
              fcwp.root_path AS root_path,
              COUNT(fcwpe.fs_content_id) AS total_count,
              SUM(CASE WHEN fsc.content IS NOT NULL THEN 1 ELSE 0 END) AS with_content,
              SUM(CASE WHEN fsc.frontmatter IS NOT NULL THEN 1 ELSE 0 END) AS with_frontmatter,
              AVG(fsc.file_bytes) AS average_size,
              strftime('%Y-%m-%d %H:%M:%S', datetime(MIN(fsc.file_mtime), 'unixepoch')) AS oldest,
              strftime('%Y-%m-%d %H:%M:%S', datetime(MAX(fsc.file_mtime), 'unixepoch')) AS youngest
          FROM
              fs_content_walk_session AS fcws
          LEFT JOIN
              fs_content_walk_path AS fcwp ON fcws.fs_content_walk_session_id = fcwp.walk_session_id
          LEFT JOIN
              fs_content_walk_path_entry AS fcwpe ON fcwp.fs_content_walk_path_id = fcwpe.walk_path_id
          LEFT JOIN
              fs_content AS fsc ON fcwpe.fs_content_id = fsc.fs_content_id
          GROUP BY
              fcws.walk_started_at,
              fcws.walk_finished_at,
              fcwpe.file_extn,
              fcwp.root_path
          UNION ALL
          SELECT
              strftime('%Y-%m-%d %H:%M:%S.%f', fcws.walk_started_at) AS walk_datetime,
              strftime('%f', fcws.walk_finished_at - fcws.walk_started_at) AS walk_duration,
              'ALL' AS file_extn,
              fcwp.root_path AS root_path,
              COUNT(fcwpe.fs_content_id) AS total_count,
              SUM(CASE WHEN fsc.content IS NOT NULL THEN 1 ELSE 0 END) AS with_content,
              SUM(CASE WHEN fsc.frontmatter IS NOT NULL THEN 1 ELSE 0 END) AS with_frontmatter,
              AVG(fsc.file_bytes) AS average_size,
              strftime('%Y-%m-%d %H:%M:%S', datetime(MIN(fsc.file_mtime), 'unixepoch')) AS oldest,
              strftime('%Y-%m-%d %H:%M:%S', datetime(MAX(fsc.file_mtime), 'unixepoch')) AS youngest
          FROM
              fs_content_walk_session AS fcws
          LEFT JOIN
              fs_content_walk_path AS fcwp ON fcws.fs_content_walk_session_id = fcwp.walk_session_id
          LEFT JOIN
              fs_content_walk_path_entry AS fcwpe ON fcwp.fs_content_walk_path_id = fcwpe.walk_path_id
          LEFT JOIN
              fs_content AS fsc ON fcwpe.fs_content_id = fsc.fs_content_id
          GROUP BY
              fcws.walk_started_at,
              fcws.walk_finished_at,
              fcwp.root_path
      )
      SELECT
          walk_datetime,
          walk_duration,
          file_extn,
          root_path,
          total_count,
          with_content,
          with_frontmatter,
          CAST(ROUND(average_size) AS INTEGER) AS average_size,
          oldest,
          youngest
      FROM
          Summary
      ORDER BY
          walk_datetime,
          file_extn`;
  }
}

/**
 * Encapsulates SQL DML and stateful table data insert/update/delete operations.
 */
export class MutationSqlNotebook<
  EmitContext extends SQLa.SqlEmitContext = SQLa.SqlEmitContext,
> extends SQLa.SqlNotebook<EmitContext> {
  constructor(readonly nbh: SqlNotebookHelpers<EmitContext>) {
    super();
  }

  mimeTypesSeedDML() {
    const { nbh, nbh: { models } } = this;
    // deno-fmt-ignore
    return nbh.SQL`
      ${nbh.loadExtnSQL("asg017/ulid/ulid0")}
      ${nbh.loadExtnSQL("asg017/http/http0")}

      -- This source: 'https://raw.githubusercontent.com/patrickmccallum/mimetype-io/master/src/mimeData.json'
      -- has the given JSON structure:
      -- [
      --   {
      --     "name": <MimeTypeName>,
      --     "description": <Description>,
      --     "types": [<Extension1>, <Extension2>, ...],
      --     "alternatives": [<Alternative1>, <Alternative2>, ...]
      --   },
      --   ...
      -- ]
      -- The goal of the SQL query is to flatten this structure into rows where each row will
      -- represent a single MIME type and one of its associated file extensions (from the 'types' array).
      -- The output will look like:
      -- | name             | description  | type         | alternatives        |
      -- |------------------|--------------|--------------|---------------------|
      -- | <MimeTypeName>   | <Description>| <Extension1> | <AlternativesArray> |
      -- | <MimeTypeName>   | <Description>| <Extension2> | <AlternativesArray> |
      -- ... and so on for all MIME types and all extensions.
      --
      -- we take all those JSON array entries and insert them into our MIME Types table
      INSERT or IGNORE INTO mime_type (mime_type_id, name, description, file_extn)
        SELECT ulid(),
               resource.value ->> '$.name' as name,
               resource.value ->> '$.description' as description,
               file_extns.value as file_extn
          FROM json_each(http_get_body('https://raw.githubusercontent.com/patrickmccallum/mimetype-io/master/src/mimeData.json')) as resource,
               json_each(resource.value, '$.types') AS file_extns;

      ${models.mimeType.insertDML({
      mime_type_id: nbh.sqlEngineNewUlid,
      name: "application/typescript",
      file_extn: ".ts",
      description: "Typescript source",
    }, { onConflict: nbh.onConflictDoNothing })};`;
  }
}

/**
 * Encapsulates SQL DQL and stateless table queries that can operate all within
 * SQLite (means they are "storable" in code_notebook_cell table).
 */
export class QuerySqlNotebook<
  EmitContext extends SQLa.SqlEmitContext = SQLa.SqlEmitContext,
> extends SQLa.SqlNotebook<EmitContext> {
  constructor(readonly nbh: SqlNotebookHelpers<EmitContext>) {
    super();
  }

  /*
   * This SQL statement retrieves column information for tables in an SQLite database
   * including table name, column ID, column name, data type, nullability, default
   * value, and primary key status.
   * It filters only tables from the result set. It is commonly used for analyzing
   * and documenting database schemas.
   * NOTE: pragma_table_info(m.tbl_name) will only work when m.type is 'table'
   * TODO: add all the same content that is emitted by infoSchemaMarkdown
   */
  infoSchema() {
    return this.nbh.SQL`
      SELECT tbl_name AS table_name,
             c.cid AS column_id,
             c.name AS column_name,
             c."type" AS "type",
             c."notnull" AS "notnull",
             c.dflt_value as "default_value",
             c.pk AS primary_key
        FROM sqlite_master m,
             pragma_table_info(m.tbl_name) c
       WHERE m.type = 'table';`;
  }

  /**
   * Generates a JSON configuration for osquery's auto_table_construction
   * feature by inspecting the SQLite database schema. The SQL creates a
   * structured JSON object detailing each table within the database. For
   * every table, the object includes a standard SELECT query, the relevant
   * columns, and the database file path.
   *
   * @example
   * // The resultant JSON object is structured as follows:
   * {
   *   "auto_table_construction": {
   *     "table_name1": {
   *       "query": "SELECT column1, column2, ... FROM table_name1",
   *       "columns": ["column1", "column2", ...],
   *       "path": "./sqlite-src.db"
   *     },
   *     ...
   *   }
   * }
   */
  infoSchemaOsQueryATCs() {
    return this.nbh.SQL`
      WITH table_columns AS (
          SELECT m.tbl_name AS table_name,
                 group_concat(c.name) AS column_names_for_select,
                 json_group_array(c.name) AS column_names_for_atc_json
            FROM sqlite_master m,
                 pragma_table_info(m.tbl_name) c
           WHERE m.type = 'table'
        GROUP BY m.tbl_name
      ),
      target AS (
        -- set SQLite parameter :osquery_atc_path to assign a different path
        SELECT COALESCE(:osquery_atc_path, 'SQLITEDB_PATH') AS path
      ),
      table_query AS (
          SELECT table_name,
                 'SELECT ' || column_names_for_select || ' FROM ' || table_name AS query,
                 column_names_for_atc_json
            FROM table_columns
      )
      SELECT json_object('auto_table_construction',
                json_group_object(
                    table_name,
                    json_object(
                        'query', query,
                        'columns', json(column_names_for_atc_json),
                        'path', path
                    )
                )
             ) AS osquery_auto_table_construction
        FROM table_query, target;`;
  }

  /**
   * SQL which generates the Markdown content lines (rows) which describes all
   * the tables, columns, indexes, and views in the database. This should really
   * be a view instead of a query but SQLite does not support use of pragma_* in
   * views for security reasons.
   * TODO: check out https://github.com/k1LoW/tbls and make this query equivalent
   *       to that utility's output including generating PlantUML through SQL.
   */
  infoSchemaMarkdown() {
    return this.nbh.SQL`
      -- TODO: https://github.com/lovasoa/SQLpage/discussions/109#discussioncomment-7359513
      --       see the above for how to fix for SQLPage but figure out to use the same SQL
      --       in and out of SQLPage (maybe do what Ophir said in discussion and create
      --       custom output for SQLPage using componetns?)
      WITH TableInfo AS (
        SELECT
          m.tbl_name AS table_name,
          CASE WHEN c.pk THEN '*' ELSE '' END AS is_primary_key,
          c.name AS column_name,
          c."type" AS column_type,
          CASE WHEN c."notnull" THEN '*' ELSE '' END AS not_null,
          COALESCE(c.dflt_value, '') AS default_value,
          COALESCE((SELECT pfkl."table" || '.' || pfkl."to" FROM pragma_foreign_key_list(m.tbl_name) AS pfkl WHERE pfkl."from" = c.name), '') as fk_refs,
          ROW_NUMBER() OVER (PARTITION BY m.tbl_name ORDER BY c.cid) AS row_num
        FROM sqlite_master m JOIN pragma_table_info(m.tbl_name) c ON 1=1
        WHERE m.type = 'table'
        ORDER BY table_name, row_num
      ),
      Views AS (
        SELECT '## Views ' AS markdown_output
        UNION ALL
        SELECT '| View | Column | Type |' AS markdown_output
        UNION ALL
        SELECT '| ---- | ------ |----- |' AS markdown_output
        UNION ALL
        SELECT '| ' || tbl_name || ' | ' || c.name || ' | ' || c."type" || ' | '
        FROM
          sqlite_master m,
          pragma_table_info(m.tbl_name) c
        WHERE
          m.type = 'view'
      ),
      Indexes AS (
        SELECT '## Indexes' AS markdown_output
        UNION ALL
        SELECT '| Table | Index | Columns |' AS markdown_output
        UNION ALL
        SELECT '| ----- | ----- | ------- |' AS markdown_output
        UNION ALL
        SELECT '| ' ||  m.name || ' | ' || il.name || ' | ' || group_concat(ii.name, ', ') || ' |' AS markdown_output
        FROM sqlite_master as m,
          pragma_index_list(m.name) AS il,
          pragma_index_info(il.name) AS ii
        WHERE
          m.type = 'table'
        GROUP BY
          m.name,
          il.name
      )
      SELECT
          markdown_output AS info_schema_markdown
      FROM
        (
          SELECT '## Tables' AS markdown_output
          UNION ALL
          SELECT
            CASE WHEN ti.row_num = 1 THEN '
      ### \`' || ti.table_name || '\` Table
      | PK | Column | Type | Req? | Default | References |
      | -- | ------ | ---- | ---- | ------- | ---------- |
      ' ||
              '| ' || is_primary_key || ' | ' || ti.column_name || ' | ' || ti.column_type || ' | ' || ti.not_null || ' | ' || ti.default_value || ' | ' || ti.fk_refs || ' |'
            ELSE
              '| ' || is_primary_key || ' | ' || ti.column_name || ' | ' || ti.column_type || ' | ' || ti.not_null || ' | ' || ti.default_value || ' | ' || ti.fk_refs || ' |'
            END
          FROM TableInfo ti
          UNION ALL SELECT ''
          UNION ALL SELECT * FROM	Views
          UNION ALL SELECT ''
          UNION ALL SELECT * FROM Indexes
      );`;
  }

  htmlAnchors() {
    // deno-fmt-ignore
    return this.nbh.SQL`
        ${this.nbh.loadExtnSQL("asg017/html/html0")}

        -- find all HTML files in the fs_content table and return
        -- each file and the anchors' labels and hrefs in that file
        -- TODO: create a table called fs_content_html_anchor to store this data after inserting it into fs_content
        --       so that simple HTML lookups do not require the html0 extension to be loaded
        WITH html_content AS (
          SELECT fs_content_id, content, content_digest, file_path, file_extn FROM fs_content WHERE file_extn = '.html'
        ),
        html AS (
          SELECT file_path,
                 text as label,
                 html_attribute_get(html, 'a', 'href') as href
            FROM html_content, html_each(html_content.content, 'a')
        )
        SELECT * FROM html;
      `;
  }

  htmlHeadMeta() {
    // deno-fmt-ignore
    return this.nbh.SQL`
        ${this.nbh.loadExtnSQL("asg017/html/html0")}

        -- find all HTML files in the fs_content table and return
        -- each file and the <head><meta name="key" content="value"> pair
        -- TODO: create a table called fs_content_html_head_meta to store this data after inserting it into fs_content
        --       so that simple HTML lookups do not require the html0 extension to be loaded
        WITH html_content AS (
          SELECT fs_content_id, content, content_digest, file_path, file_extn FROM fs_content WHERE file_extn = '.html'
        ),
        html AS (
          SELECT file_path,
                 html_attribute_get(html, 'meta', 'name') as key,
                 html_attribute_get(html, 'meta', 'content') as value,
                 html
            FROM html_content, html_each(html_content.content, 'head meta')
           WHERE key IS NOT NULL
        )
        SELECT * FROM html;
      `;
  }
}

/**
 * Encapsulates SQL needed as part of multi-language (e.g. SQLite + Deno)
 * orchestration because SQLite could not operate on its own.
 */
export class PolyglotSqlNotebook<
  EmitContext extends SQLa.SqlEmitContext = SQLa.SqlEmitContext,
> extends SQLa.SqlNotebook<EmitContext> {
  constructor(readonly nbh: SqlNotebookHelpers<EmitContext>) {
    super();
  }

  /**
   * Factory method to construct Command instances that will find all
   * frontmatter candidates and update their fs_content table rows with
   * parsed frontmatter.
   *
   * - First sqlite3 command finds all frontmatter candidates in DB, returning
   *   SQL result as JSON rows array to STDOUT without any DB side effects.
   * - Second sqlite3 command generates `UPDATE` SQL DML for all frontmatter
   *   candidates and then executes the DML from Pass 2 which mutates the
   *   database.
   *
   * If you want to capture any SQL or spawn logs, use Command loggers.
   *
   * @param sqliteDb the SQLite database to operate on
   * @returns Command pattern instances which have promises that need to be resolved
   */
  frontmatterMutationCommands(sqliteDb: string) {
    return cmdNB.sqlite3({ filename: sqliteDb })
      .SQL(`SELECT fs_content_id, content
            FROM fs_content
           WHERE (file_extn = '.md' OR file_extn = '.mdx')
             AND content IS NOT NULL
             AND content_fm_body_attrs IS NULL
             AND frontmatter IS NULL;`)
      .outputJSON() // adds "--json" arg to SQLite pass 2
      .pipe(cmdNB.sqlite3({ filename: sqliteDb }), {
        // stdIn will be called with STDOUT from previous SQL statement;
        // we will take result of `SELECT fs_content_id, content...` and
        // convert it JSON then loop through each row to prepare SQL
        // DML (`UPDATE`) to set the frontmatter. The function prepares the
        // SQL and hands it to a SQLite shell for execution.
        stdIn: this.nbh.pipeInSpawnedRows<
          { fs_content_id: string; content: string }
        > // deno-lint-ignore require-await
          (async (rows, emitStdIn, nbh) => {
            const { quotedLiteral } = nbh.emitCtx.sqlTextEmitOptions;
            rows.forEach((row) => {
              if (fm.test(row.content)) {
                const parsedFM = fm.extract(row.content);
                // each write() call adds content into the SQLite stdin
                // stream that will be sent to Deno.Command
                // deno-fmt-ignore
                emitStdIn(`UPDATE fs_content SET
                         frontmatter = ${quotedLiteral(JSON.stringify(parsedFM.attrs))[1]},
                         content_fm_body_attrs = ${quotedLiteral(JSON.stringify(parsedFM))[1]}
                       WHERE fs_content_id = '${row.fs_content_id}';\n`);
              }
            });
          }),
      });
  }
}

/**
 * Encapsulates [SQLPage](https://sql.ophir.dev/) content. SqlPageNotebook has
 * methods with the name of each [SQLPage](https://sql.ophir.dev/) content that
 * we want in the database. The MutationSqlNotebook sqlPageSeedDML method
 * "reads" the cells in the SqlPageNotebook (each method's result) and
 * generates SQL to insert the content of the page in the database in the format
 * and table expected by [SQLPage](https://sql.ophir.dev/).
 * NOTE: we break our PascalCase convention for the name of the class since SQLPage
 *       is a proper noun (product name).
 */
export class SQLPageNotebook<
  EmitContext extends SQLa.SqlEmitContext = SQLa.SqlEmitContext,
> extends SQLa.SqlNotebook<EmitContext> {
  // if you want to add any annotations, use this like:
  //   @SQLPageNotebook.nbd.init(), .finalize(), etc.
  //   @SQLPageNotebook.nbd.disregard(), etc.
  static nbd = new chainNB.NotebookDescriptor<
    SQLPageNotebook<Any>,
    chainNB.NotebookCell<
      SQLPageNotebook<Any>,
      chainNB.NotebookCellID<SQLPageNotebook<Any>>
    >
  >();
  readonly queryNB: QuerySqlNotebook<EmitContext>;

  constructor(readonly nbh: SqlNotebookHelpers<EmitContext>) {
    super();
    this.queryNB = new QuerySqlNotebook(this.nbh);
  }

  "index.sql"() {
    return this.nbh.SQL`
      SELECT
        'list' as component,
        'Get started: where to go from here ?' as title,
        'Here are some useful links to get you started with SQLPage.' as description;
      SELECT 'Content Walk Session Statistics' as title,
        'fsc-walk-session-stats.sql' as link,
        'TODO' as description,
        'green' as color,
        'download' as icon;
      SELECT 'MIME Types' as title,
        'mime-types.sql' as link,
        'TODO' as description,
        'blue' as color,
        'download' as icon;
      SELECT 'Stored SQL Notebooks' as title,
        'notebooks.sql' as link,
        'TODO' as description,
        'blue' as color,
        'download' as icon;
      SELECT 'Information Schema' as title,
        'info-schema.sql' as link,
        'TODO' as description,
        'blue' as color,
        'download' as icon;`;
  }

  "fsc-walk-session-stats.sql"() {
    return this.nbh.SQL`
      SELECT 'table' as component, 1 as search, 1 as sort;
      SELECT walk_datetime, file_extn, total_count, with_content, with_frontmatter, average_size from fs_content_walk_session_stats;`;
  }

  "mime-types.sql"() {
    return this.nbh.SQL`
      SELECT 'table' as component, 1 as search, 1 as sort;
      SELECT name, file_extn, description from mime_type;`;
  }

  "notebooks.sql"() {
    const { codeNbModels: { codeNotebookCell: cnbc } } = this.nbh.models;
    const { symbol: scnbc } = cnbc.columnNames(this.nbh.emitCtx);

    return this.nbh.SQL`
      SELECT 'table' as component, 'Cell' as markdown, 1 as search, 1 as sort;
      SELECT ${scnbc.notebook_name},
             '[' || ${scnbc.cell_name} || '](notebook-cell.sql?notebook=' ||  ${scnbc.notebook_name} || '&cell=' || ${scnbc.cell_name} || ')' as Cell
        FROM ${cnbc.tableName};`;
  }

  "notebook-cell.sql"() {
    const { codeNbModels: { codeNotebookCell: cnbc } } = this.nbh.models;
    const { symbol: scnbc } = cnbc.columnNames(this.nbh.emitCtx);

    return this.nbh.SQL`
      SELECT 'text' as component,
             $notebook || '.' || $cell as title,
             '\`\`\`sql
      ' || ${scnbc.interpretable_code} || '
      \`\`\`' as contents_md
       FROM ${cnbc.tableName}
      WHERE ${scnbc.notebook_name} = $notebook
        AND ${scnbc.cell_name} = $cell;`;
  }

  "info-schema.sql"() {
    return this.nbh.SQL`
      ${this.queryNB.infoSchemaMarkdown()}

      -- :info_schema_markdown should be defined in the above query
      SELECT 'text' as component,
             'Information Schema' as title,
             :info_schema_markdown as contents_md`;
  }

  "bad-item.sql"() {
    return "this is not a proper return type in SQLPageNotebook so it should generate an alert page in SQLPage (included just for testing)";
  }

  @SQLPageNotebook.nbd.disregard()
  "disregarded.sql"() {
    return "this should be disregarded and not included in SQLPage (might be a support function)";
  }

  // TODO: add one or more pages that will contain PlantUML or database
  //       description markdown so that the documentation for the database
  //       is contained within the DB itself.

  static create<EmitContext extends SQLa.SqlEmitContext>(
    nbh: SqlNotebookHelpers<EmitContext>,
  ) {
    const kernel = chainNB.ObservableKernel.create(
      SQLPageNotebook.prototype,
      SQLPageNotebook.nbd,
    );
    const instance = new SQLPageNotebook(nbh);
    return {
      kernel,
      instance,
      SQL: async () => {
        const irs = await kernel.initRunState();
        const { model: gm, domains: gd, keys: gk } = nbh.modelsGovn;
        const sqlPageFiles = gm.table("sqlpage_files", {
          path: gk.textPrimaryKey(),
          contents: gd.text(),
          last_modified: gd.createdAt(),
        }, { isIdempotent: true });
        const ctx = nbh.emitCtx;
        const seedSQL: SQLa.SqlTextSupplier<EmitContext>[] = [sqlPageFiles];
        irs.runState.eventEmitter.afterCell = (cell, state) => {
          if (state.status == "successful") {
            seedSQL.push(sqlPageFiles.insertDML({
              path: cell, // the class's method name is the "cell"
              // deno-fmt-ignore
              contents: SQLa.isSqlTextSupplier<EmitContext>(state.execResult)
                ? state.execResult.SQL(ctx)
                : `select 'alert' as component,
                            'MutationSqlNotebook.SQLPageSeedDML() issue' as title,
                            'SQLPageNotebook cell "${cell}" did not return SQL (found: ${typeof state.execResult})' as description;`,
              last_modified: nbh.sqlEngineNow,
            }, {
              onConflict: {
                SQL: () =>
                  `ON CONFLICT(path) DO UPDATE SET contents = EXCLUDED.contents, last_modified = CURRENT_TIMESTAMP`,
              },
            }));
          }
        };

        await kernel.run(instance, irs);
        return seedSQL;
      },
    };
  }
}

export class AssuranceSqlNotebook<
  EmitContext extends SQLa.SqlEmitContext = SQLa.SqlEmitContext,
> extends SQLa.SqlNotebook<EmitContext> {
  readonly queryNB: QuerySqlNotebook<EmitContext>;

  constructor(readonly nbh: SqlNotebookHelpers<EmitContext>) {
    super();
    this.queryNB = new QuerySqlNotebook(this.nbh);
  }

  test1() {
    return this.nbh.SQL`
      WITH test_plan AS (
          SELECT '1..1' AS tap_output
      ),
      test1 AS (  -- Check if the 'fileio' extension is loaded by calling the 'readfile' function
          SELECT
              CASE
                  WHEN readfile('README.md') IS NOT NULL THEN 'ok 1 - fileio extension is loaded.'
                  ELSE 'not ok 1 - fileio extension is not loaded.'
              END AS tap_output
          FROM (SELECT 1) -- This is a dummy table of one row to ensure the SELECT runs.
      )
      SELECT tap_output FROM test_plan
      UNION ALL
      SELECT tap_output FROM test1;`;
  }
}

export const orchestrableSqlNotebooksNames = [
  "construction",
  "mutation",
  "query",
  "polyglot",
  "assurance",
] as const;

export type OrchestrableSqlNotebookName =
  typeof orchestrableSqlNotebooksNames[number];

/**
 * Encapsulates instances of all the other notebooks and provides performs all
 * the work of creating other notebook kernel factories and actually performing
 * operations with those notebooks' cells.
 */
export class SqlNotebooksOrchestrator<
  EmitContext extends SQLa.SqlEmitContext = SQLa.SqlEmitContext,
> {
  readonly constructionNBF: ReturnType<
    typeof SQLa.sqlNotebookFactory<ConstructionSqlNotebook, EmitContext>
  >;
  readonly mutationNBF: ReturnType<
    typeof SQLa.sqlNotebookFactory<MutationSqlNotebook, EmitContext>
  >;
  readonly queryNBF: ReturnType<
    typeof SQLa.sqlNotebookFactory<QuerySqlNotebook, EmitContext>
  >;
  readonly polyglotNBF: ReturnType<
    typeof SQLa.sqlNotebookFactory<PolyglotSqlNotebook, EmitContext>
  >;
  readonly assuranceNBF: ReturnType<
    typeof SQLa.sqlNotebookFactory<AssuranceSqlNotebook, EmitContext>
  >;
  readonly constructionNB: ConstructionSqlNotebook;
  readonly mutationNB: MutationSqlNotebook;
  readonly queryNB: QuerySqlNotebook;
  readonly polyglotNB: PolyglotSqlNotebook;
  readonly assuranceNB: AssuranceSqlNotebook;

  constructor(readonly nbh: SqlNotebookHelpers<EmitContext>) {
    this.constructionNBF = SQLa.sqlNotebookFactory(
      ConstructionSqlNotebook.prototype,
      () => new ConstructionSqlNotebook<EmitContext>(nbh, []),
    );
    this.mutationNBF = SQLa.sqlNotebookFactory(
      MutationSqlNotebook.prototype,
      () => new MutationSqlNotebook<EmitContext>(nbh),
    );
    this.queryNBF = SQLa.sqlNotebookFactory(
      QuerySqlNotebook.prototype,
      () => new QuerySqlNotebook<EmitContext>(nbh),
    );
    this.polyglotNBF = SQLa.sqlNotebookFactory(
      PolyglotSqlNotebook.prototype,
      () => new PolyglotSqlNotebook<EmitContext>(nbh),
    );
    this.assuranceNBF = SQLa.sqlNotebookFactory(
      AssuranceSqlNotebook.prototype,
      () => new AssuranceSqlNotebook<EmitContext>(nbh),
    );

    this.constructionNB = this.constructionNBF.instance();
    this.mutationNB = this.mutationNBF.instance();
    this.queryNB = this.queryNBF.instance();
    this.polyglotNB = this.polyglotNBF.instance();
    this.assuranceNB = this.assuranceNBF.instance();
  }

  separator(cell: string) {
    return {
      executeSqlBehavior: () => ({
        SQL: () => `\n---\n--- Cell: ${cell}\n---\n`,
      }),
    };
  }

  infoSchemaDiagram() {
    const { nbh: { modelsGovn, models } } = this;
    const ctx = modelsGovn.sqlEmitContext();
    return typical.diaPUML.plantUmlIE(ctx, function* () {
      for (const table of models.informationSchema.tables) {
        if (SQLa.isGraphEntityDefinitionSupplier(table)) {
          yield table.graphEntityDefn() as Any; // TODO: why is "Any" required here???
        }
      }
    }, typical.diaPUML.typicalPlantUmlIeOptions()).content;
  }

  async infoSchemaDiagramDML() {
    const { nbh: { models } } = this;
    const { codeNbModels: { codeNotebookCell } } = models;
    const interpretable_code = this.infoSchemaDiagram();
    return codeNotebookCell.insertDML({
      code_notebook_cell_id: this.nbh.newUlid(),
      notebook_kernel_id: "PlantUML",
      notebook_name: SqlNotebooksOrchestrator.prototype.constructor.name,
      cell_name: "infoSchemaDiagram",
      interpretable_code,
      interpretable_code_hash: await gitLikeHash(interpretable_code),
    }, {
      onConflict: this.nbh
        .SQL`ON CONFLICT(notebook_name, cell_name, interpretable_code_hash) DO UPDATE SET
        interpretable_code = EXCLUDED.interpretable_code,
        notebook_kernel_id = EXCLUDED.notebook_kernel_id,
        updated_at = CURRENT_TIMESTAMP,
        activity_log = ${codeNotebookCell.activityLogDmlPartial()}`,
    });
  }

  introspectedCells() {
    const cells: {
      readonly notebook: OrchestrableSqlNotebookName;
      readonly cell: string;
    }[] = [];
    this.constructionNBF.kernel.introspectedNB.cells.forEach((cell) => {
      cells.push({ notebook: "construction", cell: cell.nbCellID });
    });
    this.mutationNBF.kernel.introspectedNB.cells.forEach((cell) => {
      cells.push({ notebook: "mutation", cell: cell.nbCellID });
    });
    this.queryNBF.kernel.introspectedNB.cells.forEach((cell) => {
      cells.push({ notebook: "query", cell: cell.nbCellID });
    });
    this.polyglotNBF.kernel.introspectedNB.cells.forEach((cell) => {
      cells.push({ notebook: "polyglot", cell: cell.nbCellID });
    });
    this.assuranceNBF.kernel.introspectedNB.cells.forEach((cell) => {
      cells.push({ notebook: "assurance", cell: cell.nbCellID });
    });
    return cells;
  }

  async storeNotebookCellsDML() {
    const { codeNbModels: { codeNotebookCell } } = this.nbh.models;
    const ctx = this.nbh.modelsGovn.sqlEmitContext<EmitContext>();
    const sqlDML: SQLa.SqlTextSupplier<EmitContext>[] = [];

    const kernelDML = async <
      Factory extends ReturnType<
        typeof SQLa.sqlNotebookFactory<Any, EmitContext>
      >,
    >(f: Factory, notebookName: string) => {
      // prepare the run state with list of all pages defined and have the kernel
      // traverse the cells and emit (the SQL generator, no SQL is executed)
      const instance = f.instance();
      const irs = await f.kernel.initRunState();
      irs.runState.eventEmitter.afterCell = async (cell, state) => {
        if (state.status == "successful") {
          const interpretable_code =
            SQLa.isSqlTextSupplier<EmitContext>(state.execResult)
              ? state.execResult.SQL(ctx)
              : `storeNotebookCellsDML "${cell}" did not return SQL (found: ${typeof state
                .execResult})`;
          sqlDML.push(codeNotebookCell.insertDML({
            code_notebook_cell_id: this.nbh.newUlid(),
            notebook_kernel_id: "SQL",
            notebook_name: notebookName,
            cell_name: cell, // the class's method name is the "cell"
            interpretable_code,
            interpretable_code_hash: await gitLikeHash(interpretable_code),
          }, {
            onConflict: this.nbh
              .SQL`ON CONFLICT(notebook_name, cell_name, interpretable_code_hash) DO UPDATE SET
            interpretable_code = EXCLUDED.interpretable_code,
            notebook_kernel_id = EXCLUDED.notebook_kernel_id,
            updated_at = CURRENT_TIMESTAMP,
            activity_log = ${codeNotebookCell.activityLogDmlPartial()}`,
          }));
        }
      };
      await f.kernel.run(instance, irs);
    };

    await kernelDML(
      this.constructionNBF as Any,
      ConstructionSqlNotebook.prototype.constructor.name,
    );
    await kernelDML(
      this.mutationNBF as Any,
      MutationSqlNotebook.prototype.constructor.name,
    );
    await kernelDML(
      this.queryNBF as Any,
      QuerySqlNotebook.prototype.constructor.name,
    );
    await kernelDML(
      this.assuranceNBF as Any,
      AssuranceSqlNotebook.prototype.constructor.name,
    );

    // NOTE: PolyglotSqlNotebook is not stored since its cells cannot be
    //       executed without orchestration externally
    // NOTE: SQLPageNotebook is not stored since its cells are stored in special
    //       sqlpage_files table

    return this.nbh.SQL`
      ${sqlDML};

      ${await this.infoSchemaDiagramDML()}
      `;
  }
}
