import { api, toPgError } from './api';
import type { PgError, Profile, ProfileInput } from './types';

/** Saved connections, as listed on the connect screen. */
class ProfileList {
  items = $state.raw<Profile[]>([]);
  loaded = $state(false);
  /** Why the list could not be loaded (damaged file, ...). */
  error = $state.raw<PgError | null>(null);

  async load() {
    try {
      this.items = await api.listProfiles();
      this.error = null;
    } catch (e) {
      this.items = [];
      this.error = toPgError(e);
    } finally {
      this.loaded = true;
    }
  }

  async save(input: ProfileInput): Promise<Profile> {
    const saved = await api.saveProfile(input);
    await this.load();
    return saved;
  }

  async remove(id: string) {
    await api.deleteProfile(id);
    await this.load();
  }
}

export const profiles = new ProfileList();
