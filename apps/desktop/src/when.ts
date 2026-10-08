/// A command's `when` (`.context/commands.md` "Offering a command to a
/// person"): VS Code's when-clause syntax over the window's context keys —
/// `!`, `&&` (tighter than `||`), `||`, parentheses, `==` / `!=` against
/// `true`, `false` or a string (quoted or bare), `=~` against a regular
/// expression literal. The ordering comparisons and `in` / `not in` are
/// VS Code's too but no key holds a number or a list, so they're refused.
///
/// The daemon checks each `when` where its command registers
/// (`oxplow_domain::when`, the same grammar); both read
/// `crates/oxplow-domain/fixtures/when_cases.json` alike.

/** What the window holds for each context key (`oxplow_domain::when::CONTEXT_KEYS`). */
export type WhenContext = Record<string, boolean | string>;

export type WhenExpr =
  | { kind: "key"; key: string }
  | { kind: "not"; expr: WhenExpr }
  | { kind: "and" | "or"; left: WhenExpr; right: WhenExpr }
  | { kind: "equals"; key: string; value: boolean | string; equal: boolean }
  | { kind: "matches"; key: string; regex: RegExp };

type Token =
  | { t: "(" | ")" | "!" | "&&" | "||" | "==" | "!=" | "=~" }
  | { t: "unsupported"; op: string }
  | { t: "word"; v: string }
  | { t: "quoted"; v: string }
  | { t: "regex"; pattern: string; flags: string };

const isWord = (c: string | undefined) => c !== undefined && /[\p{L}\p{N}._\-:/@#]/u.test(c);

function describe(t: Token): string {
  switch (t.t) {
    case "unsupported":
      return `\`${t.op}\``;
    case "word":
      return `\`${t.v}\``;
    case "quoted":
      return `'${t.v}'`;
    case "regex":
      return `/${t.pattern}/${t.flags}`;
    default:
      return `\`${t.t}\``;
  }
}

function tokens(src: string): Token[] {
  const chars = [...src];
  const out: Token[] = [];
  let i = 0;
  const nextWord = (from: number) => {
    let j = from;
    while (chars[j] === " ") j++;
    let w = "";
    while (isWord(chars[j])) w += chars[j++];
    return w;
  };
  while (i < chars.length) {
    const c = chars[i];
    const next = chars[i + 1];
    if (c === " " || c === "\t" || c === "\n") {
      i++;
    } else if (c === "(" || c === ")") {
      out.push({ t: c });
      i++;
    } else if (c === "&" && next === "&") {
      out.push({ t: "&&" });
      i += 2;
    } else if (c === "|" && next === "|") {
      out.push({ t: "||" });
      i += 2;
    } else if (c === "=" && next === "=") {
      out.push({ t: "==" });
      i += chars[i + 2] === "=" ? 3 : 2;
    } else if (c === "=" && next === "~") {
      out.push({ t: "=~" });
      i += 2;
      while (chars[i] === " ") i++;
      if (chars[i] !== "/") throw new Error("`=~` takes a regular expression, `/pattern/flags`");
      i++;
      let pattern = "";
      for (;;) {
        const ch = chars[i];
        if (ch === undefined) throw new Error("unterminated regular expression");
        if (ch === "\\") {
          const n = chars[i + 1];
          if (n !== undefined) {
            // `\/` is a `/` in the pattern.
            if (n !== "/") pattern += "\\";
            pattern += n;
          }
          i += 2;
        } else if (ch === "/") {
          i++;
          break;
        } else {
          pattern += ch;
          i++;
        }
      }
      let flags = "";
      while (chars[i] !== undefined && /[a-zA-Z]/.test(chars[i])) flags += chars[i++];
      out.push({ t: "regex", pattern, flags });
    } else if (c === "!" && next === "=") {
      out.push({ t: "!=" });
      i += chars[i + 2] === "=" ? 3 : 2;
    } else if (c === "!") {
      out.push({ t: "!" });
      i++;
    } else if (c === "<" || c === ">") {
      const op = next === "=" ? `${c}=` : c;
      out.push({ t: "unsupported", op });
      i += op.length;
    } else if (c === "'" || c === '"') {
      let s = "";
      i++;
      for (;;) {
        const ch = chars[i];
        if (ch === undefined) throw new Error("unterminated string");
        i++;
        if (ch === c) break;
        s += ch;
      }
      out.push({ t: "quoted", v: s });
    } else if (isWord(c)) {
      let w = "";
      while (isWord(chars[i])) w += chars[i++];
      if (w === "in") out.push({ t: "unsupported", op: "in" });
      else if (w === "not" && nextWord(i) === "in") {
        while (chars[i] === " ") i++;
        i += 2;
        out.push({ t: "unsupported", op: "not in" });
      } else out.push({ t: "word", v: w });
    } else {
      throw new Error(`unexpected \`${c}\``);
    }
  }
  return out;
}

/** `pattern` with `flags` (`i`, `m`, `s`) as a regular expression. */
function compile(pattern: string, flags: string): RegExp {
  const bad = [...flags].find((f) => !"ims".includes(f));
  if (bad) throw new Error(`regular expression flag \`${bad}\` isn't one of \`i\`, \`m\`, \`s\``);
  try {
    return new RegExp(pattern, flags);
  } catch (e) {
    throw new Error(`not a regular expression: ${e instanceof Error ? e.message : String(e)}`);
  }
}

/** `src` as a `when`, or an error saying why not. */
export function parseWhen(src: string): WhenExpr {
  const toks = tokens(src);
  let at = 0;
  const peek = () => toks[at];
  const take = () => toks[at++];
  const or = (): WhenExpr => {
    let left = and();
    while (peek()?.t === "||") {
      at++;
      left = { kind: "or", left, right: and() };
    }
    return left;
  };
  const and = (): WhenExpr => {
    let left = unary();
    while (peek()?.t === "&&") {
      at++;
      left = { kind: "and", left, right: unary() };
    }
    return left;
  };
  const unary = (): WhenExpr => {
    if (peek()?.t === "!") {
      at++;
      return { kind: "not", expr: unary() };
    }
    return primary();
  };
  const primary = (): WhenExpr => {
    const t = take();
    if (t === undefined) throw new Error("expected a context key");
    if (t.t === "(") {
      const inner = or();
      const close = take();
      if (close === undefined) throw new Error("expected `)`");
      if (close.t !== ")") throw new Error(`expected \`)\`, found ${describe(close)}`);
      return inner;
    }
    if (t.t !== "word") throw new Error(`expected a context key, found ${describe(t)}`);
    return comparison(t.v);
  };
  const comparison = (key: string): WhenExpr => {
    const op = peek();
    if (op?.t === "==" || op?.t === "!=") {
      at++;
      const v = take();
      if (v === undefined) throw new Error("expected a value");
      if (v.t !== "word" && v.t !== "quoted") throw new Error(`expected a value, found ${describe(v)}`);
      const value = v.t === "word" && v.v === "true" ? true : v.t === "word" && v.v === "false" ? false : v.v;
      return { kind: "equals", key, value, equal: op.t === "==" };
    }
    if (op?.t === "=~") {
      at++;
      const r = take();
      if (r?.t !== "regex") throw new Error("`=~` takes a regular expression, `/pattern/flags`");
      return { kind: "matches", key, regex: compile(r.pattern, r.flags) };
    }
    if (op?.t === "unsupported") {
      throw new Error(`\`${op.op}\` is not supported yet: no context key holds a number or a list`);
    }
    return { kind: "key", key };
  };
  const expr = or();
  const rest = take();
  if (rest !== undefined) throw new Error(`unexpected ${describe(rest)}`);
  return expr;
}

/** Whether `expr` holds in `context`: a key the window doesn't hold is
 *  false alone, equal to nothing (but `false`) and unmatched. */
export function whenHolds(expr: WhenExpr, context: WhenContext): boolean {
  switch (expr.kind) {
    case "key": {
      const v = context[expr.key];
      return v === true || (typeof v === "string" && v !== "");
    }
    case "not":
      return !whenHolds(expr.expr, context);
    case "and":
      return whenHolds(expr.left, context) && whenHolds(expr.right, context);
    case "or":
      return whenHolds(expr.left, context) || whenHolds(expr.right, context);
    case "equals": {
      const v = context[expr.key];
      const same = v === undefined ? expr.value === false : v === expr.value;
      return same === expr.equal;
    }
    case "matches": {
      const v = context[expr.key];
      return typeof v === "string" && expr.regex.test(v);
    }
  }
}

/** Whether command `when` holds now: absent, always; one that doesn't
 *  parse (the daemon refuses those, so never) is never offered. */
export function offeredWhen(when: string | null | undefined, context: WhenContext): boolean {
  if (!when) return true;
  try {
    return whenHolds(parseWhen(when), context);
  } catch {
    return false;
  }
}
