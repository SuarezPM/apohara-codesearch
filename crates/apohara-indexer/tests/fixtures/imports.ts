// Named imports

import { foo as bar, baz } from "external-package";
import * as Lodash from "lodash";

// Default import
import React from "react";
import { a, b, c } from "./module-a";
import MyClass from "./my-class";
// Namespace import
import * as Utils from "./utils";

// Side-effect import
import "polyfills";
import "./styles.css";

export { foo as bar } from "external-package";
// Re-exports
export { a, b } from "./module-a";
export * from "./re-export-all";

// Default export
export default function defaultExport() {}

// Named exports
export function namedExport() {}
export class NamedClass {}

// require() style
const fs = require("fs");
const path = require("path");
const dynamic = require("./dynamic");
