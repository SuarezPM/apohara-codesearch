// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Environment-config parsing in Go. Distinct "env"/"parse"/"default"
// vocabulary so a configuration-loading query resolves here.

package corpus

import (
	"strconv"
	"strings"
)

// Config holds parsed application settings.
type Config struct {
	Host    string
	Port    int
	Verbose bool
}

// ParsePort parses a port string, falling back to `fallback` when the value is
// empty or not a valid integer in the 1..65535 range.
func ParsePort(raw string, fallback int) int {
	n, err := strconv.Atoi(strings.TrimSpace(raw))
	if err != nil || n < 1 || n > 65535 {
		return fallback
	}
	return n
}

// ParseBool interprets common truthy spellings ("1", "true", "yes", "on")
// case-insensitively, defaulting to false on anything else.
func ParseBool(raw string) bool {
	switch strings.ToLower(strings.TrimSpace(raw)) {
	case "1", "true", "yes", "on":
		return true
	default:
		return false
	}
}

// LoadConfig assembles a Config from individual raw string fields, applying the
// per-field fallbacks.
func LoadConfig(host, port, verbose string) Config {
	return Config{
		Host:    strings.TrimSpace(host),
		Port:    ParsePort(port, 8080),
		Verbose: ParseBool(verbose),
	}
}
