#!/usr/bin/env node
// Cyclomatic-complexity gate (ported from flux-apis/scripts/cc-check.js).
//
// Uses the repo's own `typescript` compiler API for a single-parse, CC-only walk —
// far cheaper than spawning eslint per file. The counting rules mirror the algorithm
// from the `ts-complex` package (which is unusable on TS 5.x: its bundled tsutils
// silently detects no functions and returns {}), reimplemented natively here.
//
// A function's complexity starts at 1 and increments for each branching node:
//   if / for / for-in / for-of / while / do-while, ternary, catch, non-empty case,
//   and each && / || / ?? operator.
//
// Usage:
//   node scripts/cc-check.js [--max <n>] [file.ts|file.tsx ...]
// With no file args it reads newline/space-separated paths from stdin.
// Exit code 1 if any function exceeds --max (default 19). Set CC_MAX to override.

const ts = require("typescript");
const fs = require("fs");

const args = process.argv.slice(2);
let max = Number(process.env.CC_MAX ?? 19);
const files = [];
for (let i = 0; i < args.length; i++) {
  if (args[i] === "--max") {
    max = Number(args[++i]);
  } else {
    files.push(args[i]);
  }
}

function readStdin() {
  try {
    return fs.readFileSync(0, "utf8").split(/\s+/).filter(Boolean);
  } catch {
    return [];
  }
}

const targets = (files.length ? files : readStdin()).filter(
  (f) => f.endsWith(".ts") || f.endsWith(".tsx"),
);

function increasesComplexity(node) {
  switch (node.kind) {
    case ts.SyntaxKind.CaseClause:
      return node.statements.length > 0;
    case ts.SyntaxKind.CatchClause:
    case ts.SyntaxKind.ConditionalExpression:
    case ts.SyntaxKind.DoStatement:
    case ts.SyntaxKind.ForStatement:
    case ts.SyntaxKind.ForInStatement:
    case ts.SyntaxKind.ForOfStatement:
    case ts.SyntaxKind.IfStatement:
    case ts.SyntaxKind.WhileStatement:
      return true;
    case ts.SyntaxKind.BinaryExpression:
      return (
        node.operatorToken.kind === ts.SyntaxKind.BarBarToken ||
        node.operatorToken.kind === ts.SyntaxKind.AmpersandAmpersandToken ||
        node.operatorToken.kind === ts.SyntaxKind.QuestionQuestionToken // nullish-coalescing
      );
    default:
      return false;
  }
}

function isFunctionWithBody(node) {
  return (
    (ts.isFunctionDeclaration(node) ||
      ts.isFunctionExpression(node) ||
      ts.isArrowFunction(node) ||
      ts.isMethodDeclaration(node) ||
      ts.isConstructorDeclaration(node) ||
      ts.isGetAccessor(node) ||
      ts.isSetAccessor(node)) &&
    !!node.body
  );
}

function nameOf(node, source) {
  if (node.name && node.name.getText) return node.name.getText(source);
  if (ts.isConstructorDeclaration(node)) return "constructor";
  const parent = node.parent;
  if (parent && parent.name && parent.name.getText) {
    return parent.name.getText(source); // e.g. `const x = () => {}`
  }
  return "(anonymous)";
}

function analyze(filePath) {
  const text = fs.readFileSync(filePath).toString();
  const source = ts.createSourceFile(
    filePath,
    text,
    ts.ScriptTarget.Latest,
    true,
    filePath.endsWith(".tsx") ? ts.ScriptKind.TSX : ts.ScriptKind.TS,
  );
  const results = [];
  let complexity = 0;

  function visit(node) {
    if (isFunctionWithBody(node)) {
      const previous = complexity;
      complexity = 1;
      ts.forEachChild(node, visit);
      const { line } = source.getLineAndCharacterOfPosition(node.getStart());
      results.push({
        name: nameOf(node, source),
        complexity,
        line: line + 1,
      });
      complexity = previous;
    } else {
      if (increasesComplexity(node)) complexity += 1;
      ts.forEachChild(node, visit);
    }
  }

  ts.forEachChild(source, visit);
  return results;
}

let violations = 0;
for (const file of targets) {
  if (!fs.existsSync(file)) continue;
  let fns;
  try {
    fns = analyze(file);
  } catch (err) {
    console.error(`cc-check: failed to parse ${file}: ${err.message}`);
    continue;
  }
  for (const fn of fns) {
    if (fn.complexity > max) {
      violations++;
      console.error(
        `${file}:${fn.line}  ${fn.name} has a cyclomatic complexity of ${fn.complexity} (max ${max})`,
      );
    }
  }
}

if (violations > 0) {
  console.error(
    `\n✖ Complexity gate: ${violations} function(s) above ${max}. Refactor before committing.`,
  );
  process.exit(1);
}
