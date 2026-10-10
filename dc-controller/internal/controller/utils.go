/*
Copyright 2026.

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.
*/

package controller

import (
	"fmt"

	"github.com/pelletier/go-toml/v2"
	"k8s.io/apimachinery/pkg/apis/meta/v1/unstructured"
)

// parseConfigMapTOML parses the config.toml value in a ConfigMap's data.
// A nil config means the ConfigMap has no data.config.toml entry.
func parseConfigMapTOML(obj *unstructured.Unstructured) (map[string]any, error) {
	data, found, err := unstructured.NestedStringMap(obj.Object, "data")
	if err != nil {
		return nil, fmt.Errorf("reading ConfigMap data: %w", err)
	}
	if !found {
		return nil, nil
	}
	tomlText, ok := data["config.toml"]
	if !ok {
		return nil, nil
	}
	var config map[string]any
	if err := toml.Unmarshal([]byte(tomlText), &config); err != nil {
		return nil, fmt.Errorf("parsing config.toml: %w", err)
	}
	return config, nil
}

// ensureTOMLSection returns the named table of a parsed config.toml,
// creating it when the section does not exist yet.
func ensureTOMLSection(config map[string]any, key string) map[string]any {
	section, ok := config[key].(map[string]any)
	if !ok {
		section = make(map[string]any)
		config[key] = section
	}
	return section
}

// setConfigMapTOML serializes config and writes it back to the ConfigMap's
// config.toml entry.
func setConfigMapTOML(obj *unstructured.Unstructured, config map[string]any) error {
	out, err := toml.Marshal(config)
	if err != nil {
		return fmt.Errorf("marshaling config.toml: %w", err)
	}
	data, found, err := unstructured.NestedStringMap(obj.Object, "data")
	if err != nil {
		return fmt.Errorf("reading ConfigMap data: %w", err)
	}
	if !found {
		data = make(map[string]string)
	}
	data["config.toml"] = string(out)
	if err := unstructured.SetNestedStringMap(obj.Object, data, "data"); err != nil {
		return fmt.Errorf("setting ConfigMap data: %w", err)
	}
	return nil
}
