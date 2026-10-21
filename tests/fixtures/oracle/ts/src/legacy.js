const path = require("path");
const { upper } = require("./util/strings");
module.exports = function legacy() { return upper(path.sep); };
