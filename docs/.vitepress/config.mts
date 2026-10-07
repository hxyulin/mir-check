import { defineConfig } from 'vitepress';

const base = '/mir-check/';
const repository = 'https://github.com/hxyulin/mir-check';

export default defineConfig({
  title: 'mir-check',
  description: 'Panic freedom and function contracts, checked from Rust MIR.',
  base,
  cleanUrls: false,
  lastUpdated: true,
  head: [
    ['link', { rel: 'icon', type: 'image/svg+xml', href: `${base}mark.svg` }],
    ['meta', { name: 'theme-color', content: '#b65336' }],
  ],
  themeConfig: {
    logo: { light: '/mark.svg', dark: '/mark-dark.svg', alt: '' },
    nav: [
      { text: 'Guide', link: '/usage' },
      { text: 'Coverage', link: '/coverage' },
      { text: 'Examples', link: '/examples' },
      { text: 'Proofs', link: '/proofs' },
    ],
    sidebar: [
      {
        text: 'Use mir-check',
        items: [
          { text: 'Install and run', link: '/usage' },
          { text: 'Declare contracts', link: '/contracts' },
          { text: 'Real-code examples', link: '/examples' },
        ],
      },
      {
        text: 'Understand the result',
        items: [
          { text: 'Coverage and evidence', link: '/coverage' },
          { text: 'Fleet workspace survey', link: '/fleet-survey' },
          { text: 'How proofs work', link: '/proofs' },
        ],
      },
      {
        text: 'Develop the project',
        items: [
          { text: 'Development and docs', link: '/development' },
          { text: 'Analyzer redesign', link: '/analyzer-redesign' },
          { text: 'Typed storage model', link: '/storage-model' },
          { text: 'Implementation stages', link: '/stages' },
        ],
      },
    ],
    search: { provider: 'local' },
    socialLinks: [{ icon: 'github', link: repository }],
    editLink: { pattern: `${repository}/edit/main/docs/:path` },
    outline: [2, 3],
    footer: {
      message: 'Experimental side project · MIT OR Apache-2.0',
    },
  },
});
