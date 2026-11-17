"use strict";
const { readFileSync, writeFileSync: write } = require("fs");
const util = require("./util");

let counter = 0;

function bump(step = 1) {
  function inner() {
    return util.clamp(counter + step);
  }
  counter = inner();
  return counter;
}

const save = function (file, data) {
  write(file, JSON.stringify(data));
  [1, 2].forEach((n) => bump(n));
};

class Store extends util.Base {
  load(file) {
    return readFileSync(file, "utf8");
  }
  #secret() {
    return save;
  }
}

export { bump, Store as DefaultStore };
module.exports = { save };
