import fs from "node:fs";
import path from "node:path";
import { TextDecoder } from "node:util";
import ts from "../sdk/node_modules/typescript/lib/typescript.js";

const [sourceArgument, rootModule] = process.argv.slice(2);
if (!sourceArgument || !rootModule) throw new Error("usage: validate-javascript-modules.mjs SOURCE ROOT_MODULE");

const decoder = new TextDecoder("utf-8", { fatal: true });
const sourceRoot = fs.realpathSync(sourceArgument);
if (!fs.statSync(sourceRoot).isDirectory()) throw new Error("source root is not a directory");

function validateNormalizedName(name) {
  if (!name || name.startsWith("/") || name.includes("\\") || name.includes(":") || name.includes("\0")) {
    throw new Error(`invalid module path: ${name}`);
  }
  const parts = name.split("/");
  if (parts.some((part) => !part || part === "." || part === "..") || !name.endsWith(".mjs")) {
    throw new Error(`invalid module path: ${name}`);
  }
  return name;
}

function resolveContained(name) {
  const candidate = fs.realpathSync(path.join(sourceRoot, ...name.split("/")));
  const relative = path.relative(sourceRoot, candidate);
  if (relative === ".." || relative.startsWith(`..${path.sep}`) || path.isAbsolute(relative)) {
    throw new Error(`module escapes source root: ${name}`);
  }
  if (!fs.statSync(candidate).isFile()) throw new Error(`module is not a regular file: ${name}`);
  return candidate;
}

function normalizeSpecifier(referrer, specifier) {
  if (specifier === "aura:runtime") return null;
  if ((!specifier.startsWith("./") && !specifier.startsWith("../")) ||
      specifier.includes("\\") || specifier.includes(":") || specifier.includes("\0")) {
    throw new Error(`forbidden module specifier '${specifier}' in ${referrer}`);
  }
  const normalized = path.posix.normalize(path.posix.join(path.posix.dirname(referrer), specifier));
  if (normalized === ".." || normalized.startsWith("../") || normalized.startsWith("/")) {
    throw new Error(`module specifier escapes source root: ${specifier}`);
  }
  return validateNormalizedName(normalized);
}

function importsOf(moduleName, source) {
  const syntax = ts.createSourceFile(moduleName, source, ts.ScriptTarget.Latest, true, ts.ScriptKind.JS);
  if (syntax.parseDiagnostics.length !== 0) {
    throw new Error(`invalid JavaScript syntax in ${moduleName}`);
  }
  const imports = [];
  function addSpecifier(node) {
    if (!node || !ts.isStringLiteral(node)) {
      throw new Error(`module specifier must be a string literal in ${moduleName}`);
    }
    const normalized = normalizeSpecifier(moduleName, node.text);
    if (normalized !== null) imports.push(normalized);
  }
  function visit(node) {
    if (ts.isImportDeclaration(node) || ts.isExportDeclaration(node)) {
      addSpecifier(node.moduleSpecifier);
    } else if (ts.isCallExpression(node) && node.expression.kind === ts.SyntaxKind.ImportKeyword) {
      if (node.arguments.length !== 1) throw new Error(`dynamic import must have one argument in ${moduleName}`);
      addSpecifier(node.arguments[0]);
    }
    ts.forEachChild(node, visit);
  }
  visit(syntax);
  return imports;
}

const pending = [validateNormalizedName(rootModule)];
const visited = new Set();
while (pending.length !== 0) {
  const moduleName = pending.pop();
  if (visited.has(moduleName)) continue;
  const file = resolveContained(moduleName);
  const source = decoder.decode(fs.readFileSync(file));
  if (source.includes("\0")) throw new Error(`module contains NUL: ${moduleName}`);
  visited.add(moduleName);
  for (const dependency of importsOf(moduleName, source)) pending.push(dependency);
}

process.stdout.write(JSON.stringify([...visited].sort()));
