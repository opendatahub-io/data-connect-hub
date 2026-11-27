{{- define "dch.prefix" -}}
{{- .Values.namePrefix | trimSuffix "-" }}
{{- end }}

{{- define "dch.labels" -}}
app.kubernetes.io/part-of: data-connect-hub
dataconnecthub.opendatahub.io/managed-by: dataconnectservice
{{- end }}

{{- define "dch.restName" -}}
{{ include "dch.prefix" . }}-rest-service
{{- end }}

{{- define "dch.restSA" -}}
{{ include "dch.restName" . }}-sa
{{- end }}
