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
CREATE TABLE IF NOT EXISTS data_connections (
    data JSONB NOT NULL
);

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
CREATE TABLE IF NOT EXISTS data_connection_types (
    data JSONB NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_data_connection_types_name ON data_connection_types ((data->'resource'->>'name'));
CREATE INDEX IF NOT EXISTS idx_data_connection_types_provider ON data_connection_types ((data->'resource'->>'provider'));
CREATE UNIQUE INDEX IF NOT EXISTS idx_data_connection_types_name_tenant ON data_connection_types ((data->'resource'->>'name'), (data->'metadata'->>'tenant_id'));
CREATE UNIQUE INDEX IF NOT EXISTS idx_data_connection_types_id ON data_connection_types ((data->'metadata'->>'id'));
