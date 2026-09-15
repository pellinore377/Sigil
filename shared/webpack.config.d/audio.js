config.externals = Object.assign({}, config.externals, {
    './sigil_browser_audio.js': 'import /web/sigil_browser_audio.js'
});
config.output = config.output || {};
config.output.environment = Object.assign({}, config.output.environment, {
    dynamicImport: true
});
