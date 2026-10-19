import { describe, expect, it } from 'vitest';
import { diagnostics, osLabel, serverLabel, versionLabel } from './about';
import type { BuildInfo } from './types';

const release: BuildInfo = { version: '1.2.3', os: 'windows', arch: 'x86_64', debug: false };

describe('versionLabel', () => {
  it('names the app and its version', () => {
    expect(versionLabel(release)).toBe('Slon 1.2.3');
  });
  it('marks debug builds so a bug report says so', () => {
    expect(versionLabel({ ...release, debug: true })).toBe('Slon 1.2.3 (debug)');
  });
  it('falls back to the name alone before the version has loaded', () => {
    expect(versionLabel(null)).toBe('Slon');
  });
});

describe('osLabel', () => {
  it('uses the names people recognise', () => {
    expect(osLabel(release)).toBe('Windows x86_64');
    expect(osLabel({ ...release, os: 'macos', arch: 'aarch64' })).toBe('macOS aarch64');
  });
  it('passes an unknown platform through rather than hiding it', () => {
    expect(osLabel({ ...release, os: 'freebsd' })).toBe('freebsd x86_64');
  });
});

describe('serverLabel', () => {
  it('drops the platform tail PostgreSQL appends', () => {
    expect(serverLabel('16.15 (Ubuntu 16.15-0ubuntu0.24.04.1)')).toBe('PostgreSQL 16.15');
    expect(serverLabel('17.2')).toBe('PostgreSQL 17.2');
  });
  it('is absent when not connected', () => {
    expect(serverLabel(undefined)).toBeNull();
    expect(serverLabel('   ')).toBeNull();
  });
});

describe('diagnostics', () => {
  it('lists what a bug report needs', () => {
    expect(diagnostics(release, '16.15 (Debian)')).toBe('Slon 1.2.3\nWindows x86_64\nPostgreSQL 16.15');
  });
  it('leaves out the server line when not connected', () => {
    expect(diagnostics(release)).toBe('Slon 1.2.3\nWindows x86_64');
  });
  it('never carries a host, user or database name', () => {
    const text = diagnostics(release, '16.15 (Debian)');
    for (const secret of ['@', 'localhost', '5432']) expect(text).not.toContain(secret);
  });
});
