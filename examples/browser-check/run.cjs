// Runs the browser runner's cases and the Rust guest at the given path under Node, and writes
// their results to the given file.
const fs = require("node:fs");
const [, , packagePath, guestPath, outputPath] = process.argv;
const check = require(packagePath);
check.runCases(fs.readFileSync(guestPath)).then((results) => fs.writeFileSync(outputPath, results));
