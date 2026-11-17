const path = require("path");
const core = require("..");
const { greet } = require("../src/greet");

console.log(path.join(__dirname, "dist"), core);
