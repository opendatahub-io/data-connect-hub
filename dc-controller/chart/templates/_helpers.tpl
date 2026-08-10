{{- define "dc-controller.name" -}}
dc-controller
{{- end -}}

{{- define "dc-controller.fullname" -}}
dc-controller-controller-manager
{{- end -}}

{{- define "dc-controller.labels" -}}
app.kubernetes.io/name: dc-controller
app.kubernetes.io/managed-by: {{ .Release.Service }}
app.kubernetes.io/instance: {{ .Release.Name }}
control-plane: controller-manager
{{- end -}}

{{- define "dc-controller.selectorLabels" -}}
control-plane: controller-manager
app.kubernetes.io/name: dc-controller
{{- end -}}
