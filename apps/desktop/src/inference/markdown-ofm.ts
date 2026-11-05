import type { Literal, Parent, PhrasingContent } from 'mdast';
import type { Extension as FromMarkdownExtension, Handle } from 'mdast-util-from-markdown';
import type { Code, Construct, Event, Extension, Resolver, State } from 'micromark-util-types';
import type { Processor } from 'unified';

export interface OfmLiteral extends Literal {
  type: 'ofmComment' | 'ofmWiki' | 'ofmEmbed' | 'ofmFootnote' | 'ofmFootnoteReference';
}
export interface OfmHighlight extends Parent {
  type: 'ofmHighlight';
  children: PhrasingContent[];
}
export interface OfmCommentBlock extends Literal {
  type: 'ofmCommentBlock';
}

declare module 'mdast' {
  interface RootContentMap {
    ofmHighlight: OfmHighlight;
    ofmComment: OfmLiteral;
    ofmWiki: OfmLiteral;
    ofmEmbed: OfmLiteral;
    ofmFootnote: OfmLiteral;
    ofmFootnoteReference: OfmLiteral;
    ofmCommentBlock: OfmCommentBlock;
  }
  interface PhrasingContentMap {
    ofmHighlight: OfmHighlight;
    ofmComment: OfmLiteral;
    ofmWiki: OfmLiteral;
    ofmEmbed: OfmLiteral;
    ofmFootnote: OfmLiteral;
    ofmFootnoteReference: OfmLiteral;
  }
  interface BlockContentMap {
    ofmCommentBlock: OfmCommentBlock;
  }
}
declare module 'micromark-util-types' {
  interface TokenTypeMap {
    ofmHighlight: 'ofmHighlight';
    ofmComment: 'ofmComment';
    ofmCommentData: 'ofmCommentData';
    ofmWiki: 'ofmWiki';
    ofmEmbed: 'ofmEmbed';
    ofmFootnote: 'ofmFootnote';
    ofmFootnoteReference: 'ofmFootnoteReference';
    ofmCommentBlock: 'ofmCommentBlock';
    ofmProtected: 'ofmProtected';
    ofmHighlightSequence: 'ofmHighlightSequence';
    ofmHighlightMarker: 'ofmHighlightMarker';
  }
}

const isLineEnding = (code: Code) => code === -5 || code === -4 || code === -3;

/** Match highlight delimiters after code/math/link tokenizers have protected
 * their contents. A container resolver also keeps soft line breaks in one span.
 */
const resolveHighlight: Resolver = (events, context) => {
  for (let close = 0; close < events.length; ++close) {
    const closer = events[close];
    if (closer[0] !== 'enter' || closer[1].type !== 'ofmHighlightSequence' || !closer[1]._close)
      continue;
    for (let open = close - 1; open >= 0; --open) {
      const opener = events[open];
      if (opener[0] !== 'exit' || opener[1].type !== 'ofmHighlightSequence' || !opener[1]._open)
        continue;
      const token = {
        type: 'ofmHighlight' as const,
        start: { ...opener[1].start },
        end: { ...closer[1].end },
      };
      opener[1].type = 'ofmHighlightMarker';
      closer[1].type = 'ofmHighlightMarker';
      let inside = events.slice(open + 1, close);
      const applied = new Set<Resolver>();
      for (const construct of context.parser.constructs.insideSpan.null ?? []) {
        if (
          construct.resolveAll &&
          construct.resolveAll !== resolveHighlight &&
          !applied.has(construct.resolveAll)
        ) {
          applied.add(construct.resolveAll);
          inside = construct.resolveAll(inside, context);
        }
      }
      const replacement: Event[] = [
        ['enter', token, context],
        events[open - 1],
        opener,
        ...inside,
        closer,
        events[close + 1],
        ['exit', token, context],
      ];
      events.splice(open - 1, close - open + 3, ...replacement);
      close = open + replacement.length - 2;
      break;
    }
  }
  for (const event of events) if (event[1].type === 'ofmHighlightSequence') event[1].type = 'data';
  return events;
};
const highlight: Construct = {
  name: 'ofmHighlight',
  resolveAll: resolveHighlight,
  tokenize(effects, ok, nok) {
    const before = this.previous;
    let size = 0;
    let token: ReturnType<typeof effects.enter>;
    return start;
    function start(code: Code): State | undefined {
      if (before === 61) return nok(code);
      token = effects.enter('ofmHighlightSequence');
      return sequence(code);
    }
    function sequence(code: Code): State | undefined {
      if (code === 61) {
        if (++size > 2) return nok(code);
        effects.consume(code);
        return sequence;
      }
      if (size !== 2) return nok(code);
      effects.exit('ofmHighlightSequence');
      token._open =
        code !== null && !isLineEnding(code) && code !== 32 && code !== -1 && code !== -2;
      token._close =
        before !== null && !isLineEnding(before) && before !== 32 && before !== -1 && before !== -2;
      return ok(code);
    }
  },
};

function protectedSpan(marker: number): Construct {
  return {
    partial: true,
    tokenize(effects, ok, nok) {
      let opening = 0,
        closing = 0,
        size = 0;
      let escaped = false;
      return start;
      function start(code: Code): State | undefined {
        if (!opening) effects.enter('ofmProtected');
        if (code === marker) {
          ++opening;
          effects.consume(code);
          return start;
        }
        return body(code);
      }
      function body(code: Code): State | undefined {
        if (code === null || isLineEnding(code) || ++size > 65_536) return nok(code);
        if (escaped) {
          escaped = false;
          effects.consume(code);
          return body;
        }
        if (marker === 36 && code === 92) {
          escaped = true;
          effects.consume(code);
          return body;
        }
        if (code === marker) {
          closing = 1;
          effects.consume(code);
          return end;
        }
        effects.consume(code);
        return body;
      }
      function end(code: Code): State | undefined {
        if (code === marker) {
          ++closing;
          effects.consume(code);
          return end;
        }
        if (closing !== opening) return body(code);
        effects.exit('ofmProtected');
        return ok(code);
      }
    },
  };
}
const codeSpan = protectedSpan(96);
const mathSpan = protectedSpan(36);

/** OFM constructs enter at micromark's text layer, after escapes/code are resolved.
 * Capturing the complete source protects comments, wiki targets and inline notes
 * from CommonMark emphasis/link rules. Inline note bodies are parsed separately.
 */
function delimited(
  name: OfmLiteral['type'],
  opening: string,
  closing: string,
  nestedBrackets = false,
): Construct {
  return {
    name,
    tokenize(effects, ok, nok) {
      let prefix = 0,
        close = 0,
        brackets = 0,
        size = 0;
      let escaped = false;
      const protectCode = name === 'ofmFootnote';
      const maximum =
        name === 'ofmWiki' || name === 'ofmEmbed'
          ? 8192
          : name === 'ofmFootnoteReference'
            ? 2048
            : 131_072;
      return start;

      function start(code: Code): State | undefined {
        if (code !== opening.charCodeAt(prefix)) return nok(code);
        if (prefix === 0) {
          effects.enter(name);
          if (name === 'ofmComment') effects.enter('ofmCommentData');
        }
        effects.consume(code);
        ++prefix;
        return prefix === opening.length ? body : start;
      }
      function body(code: Code): State | undefined {
        if (code === null || (isLineEnding(code) && name !== 'ofmComment') || ++size > maximum)
          return nok(code);
        if (isLineEnding(code)) {
          // Give each text chunk its own data events. Otherwise a comment that
          // ends a multiline paragraph can leave a one-event chunk, making
          // micromark's subtokenizer repeatedly jump to the same event.
          effects.exit('ofmCommentData');
          effects.enter('lineEnding');
          effects.consume(code);
          effects.exit('lineEnding');
          escaped = false;
          return afterLineEnding;
        }
        if (escaped) {
          escaped = false;
          effects.consume(code);
          return body;
        }
        if (code === 92) {
          escaped = true;
          effects.consume(code);
          return body;
        }
        if (protectCode && (code === 96 || code === 36))
          return effects.attempt(code === 96 ? codeSpan : mathSpan, body, literal)(code);
        if (nestedBrackets && code === 91) {
          ++brackets;
          effects.consume(code);
          return body;
        }
        if (nestedBrackets && code === 93 && brackets) {
          --brackets;
          effects.consume(code);
          return body;
        }
        if (code === closing.charCodeAt(0)) {
          close = 1;
          effects.consume(code);
          if (close === closing.length) {
            if (name === 'ofmComment') effects.exit('ofmCommentData');
            effects.exit(name);
            return ok;
          }
          return end;
        }
        effects.consume(code);
        return body;
      }
      function afterLineEnding(code: Code): State | undefined {
        effects.enter('ofmCommentData');
        return body(code);
      }
      function literal(code: Code): State | undefined {
        effects.consume(code);
        return body;
      }
      function end(code: Code): State | undefined {
        if (code !== closing.charCodeAt(close)) return body(code);
        effects.consume(code);
        if (++close < closing.length) return end;
        if (name === 'ofmComment') effects.exit('ofmCommentData');
        effects.exit(name);
        return ok;
      }
    },
  };
}

export function remarkOfm(this: Processor): void {
  const syntax: Extension = {
    text: {
      33: delimited('ofmEmbed', '![[', ']]'),
      37: delimited('ofmComment', '%%', '%%'),
      61: highlight,
      91: [delimited('ofmWiki', '[[', ']]'), delimited('ofmFootnoteReference', '[^', ']')],
      94: delimited('ofmFootnote', '^[', ']', true),
    },
    insideSpan: { null: [highlight] },
    attentionMarkers: { null: [61] },
  };
  const enter: Record<string, Handle> = {};
  const exit: Record<string, Handle> = {};
  for (const type of [
    'ofmComment',
    'ofmWiki',
    'ofmEmbed',
    'ofmFootnote',
    'ofmFootnoteReference',
  ] as const) {
    enter[type] = function (token) {
      this.enter({ type, value: '' }, token);
    };
    exit[type] = function (token) {
      const node = this.stack.at(-1) as OfmLiteral | OfmCommentBlock;
      node.value = this.sliceSerialize(token);
      this.exit(token);
    };
  }
  enter.ofmHighlight = function (token) {
    this.enter({ type: 'ofmHighlight', children: [] }, token);
  };
  exit.ofmHighlight = function (token) {
    this.exit(token);
  };
  // remark-frontmatter supplies the fenced flow tokenizer and buffering. Keep
  // comment line boundaries instead of trimming them like a YAML document body.
  exit.ofmCommentBlock = function (token) {
    const value = this.resume();
    const node = this.stack.at(-1) as OfmCommentBlock;
    this.exit(token);
    node.value = value;
  };
  const extension: FromMarkdownExtension = { enter, exit, canContainEols: ['ofmHighlight'] };
  const data = this.data();
  (data.micromarkExtensions ??= []).push(syntax);
  (data.fromMarkdownExtensions ??= []).push(extension);
}

export function unescapeWiki(value: string): string {
  return value.replace(/\\([\\|\[\]])/g, '$1');
}

export function wikiParts(raw: string, embedded: boolean) {
  const value = unescapeWiki(raw.slice(embedded ? 3 : 2, -2));
  const separator = value.indexOf('|');
  const target = (separator < 0 ? value : value.slice(0, separator)).trim();
  let label = separator < 0 ? undefined : value.slice(separator + 1);
  const lastSeparator = label?.lastIndexOf('|') ?? -1;
  const size = embedded && label?.slice(lastSeparator + 1).match(/^(\d+)(?:x(\d+))?$/);
  if (size) label = lastSeparator < 0 ? undefined : label!.slice(0, lastSeparator);
  return {
    target,
    ...(label !== undefined ? { label } : {}),
    ...(size ? { width: Number(size[1]), ...(size[2] ? { height: Number(size[2]) } : {}) } : {}),
  };
}
