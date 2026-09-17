-- Stores DataConnectionResource records as JSONB documents.
--
-- JSONB schema (DataConnectionResource):
--   metadata                object    — ResourceMetadata
--     id                    string    — unique connection identifier (UUID)
--     tenant_id             string    — tenant this connection belongs to
--     created_at            string    — ISO 8601 creation timestamp
--     updated_at            string    — ISO 8601 last-update timestamp
--   resource                object    — DataConnection
--     name                  string    — human-readable connection name
--     data_connection_type_id string  — references data_connection_types metadata.id
--     format                string    — data format (e.g. "tabular")
--     admin                 object    — admin metadata
--       secret_ref          string    — name of the secret holding credentials
--     properties            object    — arbitrary key/value pairs
--
-- data_connection_type_id is a stored generated column mirroring
-- resource.data_connection_type_id. It exists so the reference to
-- data_connection_types can be enforced by a real foreign key (see the bottom
-- of this file); a foreign key cannot target or originate from an expression
-- index. NOT NULL matters: a NULL foreign key column is exempt from the
-- constraint, so a blob missing the key would otherwise insert as an orphan.
CREATE TABLE IF NOT EXISTS data_connections (
    data JSONB NOT NULL,
    data_connection_type_id TEXT
        GENERATED ALWAYS AS (data->'resource'->>'data_connection_type_id') STORED NOT NULL
);

-- For databases created before the generated column was introduced.
ALTER TABLE data_connections
    ADD COLUMN IF NOT EXISTS data_connection_type_id TEXT
        GENERATED ALWAYS AS (data->'resource'->>'data_connection_type_id') STORED NOT NULL;

CREATE INDEX IF NOT EXISTS idx_data_connections_tenant ON data_connections ((data->'metadata'->>'tenant_id'));
CREATE INDEX IF NOT EXISTS idx_data_connections_name ON data_connections ((data->'resource'->>'name'));
CREATE UNIQUE INDEX IF NOT EXISTS idx_data_connections_name_tenant ON data_connections ((data->'resource'->>'name'), (data->'metadata'->>'tenant_id'));
CREATE UNIQUE INDEX IF NOT EXISTS idx_data_connections_id ON data_connections ((data->'metadata'->>'id'));

-- Stores DataConnectionTypeResource records as JSONB documents.
-- Each type defines a provider (e.g. "postgres", "sqlite") and the
-- credential fields required to connect.
--
-- JSONB schema (DataConnectionTypeResource):
--   metadata                object          — ResourceMetadata
--     id                    string          — unique type identifier (UUID)
--     tenant_id             string          — tenant scope (empty = global)
--     created_at            string          — ISO 8601 creation timestamp
--     updated_at            string          — ISO 8601 last-update timestamp
--   resource                object          — DataConnectionType
--     name                  string          — display name
--     provider              string          — connector provider key
--     description           string | null   — optional description
--     credentials_fields    array of Field  — credential field definitions
--       Field:
--         name              string          — field key
--         label             string          — display label
--         description       string | null   — optional help text
--         required          boolean         — whether the field is mandatory
--         type              string          — value type (e.g. "string", "enum")
--         enum_values       array | null    — allowed values when type is "enum"
--           EnumValue:
--             value         string          — stored value
--             label         string          — display label
--         default_value     string | null   — optional default
-- id is a stored generated column mirroring metadata.id, so that it can serve
-- as the target of the data_connections foreign key.
CREATE TABLE IF NOT EXISTS data_connection_types (
    data JSONB NOT NULL,
    id TEXT GENERATED ALWAYS AS (data->'metadata'->>'id') STORED NOT NULL
);

-- For databases created before the generated column was introduced.
ALTER TABLE data_connection_types
    ADD COLUMN IF NOT EXISTS id TEXT
        GENERATED ALWAYS AS (data->'metadata'->>'id') STORED NOT NULL;

CREATE INDEX IF NOT EXISTS idx_data_connection_types_name ON data_connection_types ((data->'resource'->>'name'));
CREATE INDEX IF NOT EXISTS idx_data_connection_types_provider ON data_connection_types ((data->'resource'->>'provider'));
CREATE UNIQUE INDEX IF NOT EXISTS idx_data_connection_types_name_tenant ON data_connection_types ((data->'resource'->>'name'), (data->'metadata'->>'tenant_id'));

-- Unique index over the generated column; this is what the foreign key targets.
CREATE UNIQUE INDEX IF NOT EXISTS idx_data_connection_types_uid ON data_connection_types (id);

-- Superseded by idx_data_connection_types_uid. Dropped unconditionally rather
-- than recreated: the foreign key depends on the index above, never this one,
-- so the drop stays safe on repeat runs.
DROP INDEX IF EXISTS idx_data_connection_types_id;

-- Referential integrity between connections and their type. ON DELETE RESTRICT
-- makes deleting a type that still has connections fail with SQLSTATE 23503
-- rather than leaving those connections orphaned. PostgreSQL rejects
-- ON DELETE SET NULL/CASCADE on generated columns, so this cannot silently
-- become an orphaning cascade later.
--
-- Guarded by a catalog lookup because ADD CONSTRAINT has no IF NOT EXISTS and
-- this file is replayed on every service start.
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'fk_data_connections_type'
          AND conrelid = 'data_connections'::regclass
    ) THEN
        ALTER TABLE data_connections
            ADD CONSTRAINT fk_data_connections_type
            FOREIGN KEY (data_connection_type_id)
            REFERENCES data_connection_types (id)
            ON DELETE RESTRICT;
    END IF;
END
$$;

-- PostgreSQL does not index the referencing side of a foreign key
-- automatically; the ON DELETE RESTRICT check needs this.
CREATE INDEX IF NOT EXISTS idx_data_connections_type_id ON data_connections (data_connection_type_id);
