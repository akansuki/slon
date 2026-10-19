import { describe, expect, it } from 'vitest';
import { fmtOffset, formatAll, formatMessage, hasWarning, toneOf } from './messages';
import type { ServerMessage } from './types';

const msg = (over: Partial<ServerMessage> = {}): ServerMessage => ({
  severity: 'NOTICE',
  message: 'hello',
  detail: null,
  hint: null,
  context: null,
  sqlstate: '00000',
  elapsedMs: 0,
  ...over,
});

describe('messages', () => {
  it('prints a message the way psql does', () => {
    expect(formatMessage(msg())).toBe('NOTICE:  hello');
    expect(
      formatMessage(msg({ severity: 'WARNING', detail: 'd', hint: 'h', context: 'PL/pgSQL function f() line 3\nSQL statement' })),
    ).toBe('WARNING:  hello\nDETAIL:  d\nHINT:  h\nCONTEXT:  PL/pgSQL function f() line 3\n  SQL statement');
    expect(formatAll([msg(), msg({ message: 'two' })])).toBe('NOTICE:  hello\nNOTICE:  two');
    expect(formatAll([])).toBe('');
  });

  it('classifies severities', () => {
    expect(toneOf('WARNING')).toBe('warn');
    expect(toneOf('NOTICE')).toBe('info');
    expect(toneOf('INFO')).toBe('info');
    expect(toneOf('LOG')).toBe('dim');
    expect(toneOf('DEBUG3')).toBe('dim');
    expect(hasWarning([msg(), msg({ severity: 'WARNING' })])).toBe(true);
    expect(hasWarning([msg()])).toBe(false);
  });

  it('shows when a message arrived', () => {
    expect(fmtOffset(0)).toBe('+0 ms');
    expect(fmtOffset(350)).toBe('+350 ms');
    expect(fmtOffset(1500)).toBe('+1.50 s');
  });
});
