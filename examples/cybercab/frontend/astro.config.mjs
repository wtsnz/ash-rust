import { defineConfig } from 'astro/config';
import node from '@astrojs/node';
import react from '@astrojs/react';

// https://astro.build/config
export default defineConfig({
  output: 'server',
  adapter: node({
    mode: 'standalone',
  }),
  integrations: [react()],
  // The wall display has no room for Astro's dev toolbar.
  devToolbar: { enabled: false },
  server: {
    port: 4321,
    host: true,
  },
});
