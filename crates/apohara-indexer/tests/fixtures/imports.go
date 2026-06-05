package imports

import (
	"fmt"
	"strings"

	rename "errors"
)

func UseImports(s string) string {
	if s == "" {
		_ = rename.New("empty")
	}
	return fmt.Sprintf("%s", strings.ToUpper(s))
}
