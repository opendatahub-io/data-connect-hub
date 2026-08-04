-- Stores DataConnection records as JSONB documents.
--
-- JSONB schema:
--   id                      string    — unique connection identifier
--   name                    string    — human-readable connection name
--   data_connection_type_id string    — references data_connection_types.data->>'id'
--   format                  string    — data format (e.g. "tabular")
--   tenant_id               string    — tenant this connection belongs to
--   admin                   object    — admin metadata
--     secret_ref            string    — name of the secret holding credentials
--   created_at              string    — ISO 8601 creation timestamp
--   updated_at              string    — ISO 8601 last-update timestamp
--   properties              object    — arbitrary key/value pairs
CREATE TABLE IF NOT EXISTS data_connections (
    data JSONB NOT NULL
);

CREATE INDEX idx_data_connections_tenant ON data_connections ((data->>'tenant_id'));
CREATE INDEX idx_data_connections_name ON data_connections ((data->>'name'));

-- Stores DataConnectionType records as JSONB documents.
-- Each type defines a provider (e.g. "postgres", "sqlite") and the
-- credential fields required to connect.
--
-- JSONB schema:
--   id                  string          — unique type identifier
--   tenant_id           string | null   — optional tenant scope (null = global)
--   name                string          — display name
--   provider            string          — connector provider key (matches FlightConnector::provider())
--   description         string | null   — optional description
--   credentials_fields  array of Field  — credential field definitions
--     Field:
--       name            string          — field key
--       label           string          — display label
--       description     string | null   — optional help text
--       required        boolean         — whether the field is mandatory
--       type            string          — value type (e.g. "string", "enum")
--       enum_values     array | null    — allowed values when type is "enum"
--         EnumValue:
--           value       string          — stored value
--           label       string          — display label
--       default_value   string | null   — optional default
CREATE TABLE IF NOT EXISTS data_connection_types (
    data JSONB NOT NULL
);

CREATE INDEX idx_data_connection_types_id ON data_connection_types ((data->>'id'));
CREATE INDEX idx_data_connection_types_name ON data_connection_types ((data->>'name'));
CREATE INDEX idx_data_connection_types_provider ON data_connection_types ((data->>'provider'));
