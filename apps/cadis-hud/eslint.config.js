import js from "@eslint/js";
import tseslint from "@typescript-eslint/eslint-plugin";
import tsparser from "@typescript-eslint/parser";
import react from "eslint-plugin-react";
import reactHooks from "eslint-plugin-react-hooks";

export default [
  js.configs.recommended,
  {
    files: ["src/**/*.{ts,tsx}"],
    languageOptions: {
      parser: tsparser,
      parserOptions: {
        ecmaVersion: 2022,
        sourceType: "module",
        ecmaFeatures: { jsx: true },
      },
      globals: {
        window: "readonly",
        document: "readonly",
        console: "readonly",
        setTimeout: "readonly",
        clearTimeout: "readonly",
        setInterval: "readonly",
        clearInterval: "readonly",
        HTMLDivElement: "readonly",
        HTMLElement: "readonly",
        HTMLInputElement: "readonly",
        HTMLTextAreaElement: "readonly",
        HTMLButtonElement: "readonly",
        WebSocket: "readonly",
        // Browser audio / fetch / DOM types used by the voice stack and themes.
        AudioContext: "readonly",
        AudioBuffer: "readonly",
        Blob: "readonly",
        btoa: "readonly",
        fetch: "readonly",
        requestAnimationFrame: "readonly",
        cancelAnimationFrame: "readonly",
        SpeechSynthesisVoice: "readonly",
        SpeechSynthesisUtterance: "readonly",
        speechSynthesis: "readonly",
        Document: "readonly",
        localStorage: "readonly",
        navigator: "readonly",
        indexedDB: "readonly",
        MediaStream: "readonly",
        MediaRecorder: "readonly",
        URL: "readonly",
        FormData: "readonly",
      },
    },
    plugins: {
      "@typescript-eslint": tseslint,
      react,
      "react-hooks": reactHooks,
    },
    settings: { react: { version: "18" } },
    rules: {
      ...tseslint.configs.recommended.rules,
      ...react.configs.recommended.rules,
      ...reactHooks.configs.recommended.rules,
      "react/react-in-jsx-scope": "off",
      "react/prop-types": "off",
      "@typescript-eslint/no-unused-vars": ["error", { argsIgnorePattern: "^_" }],
      "@typescript-eslint/no-explicit-any": "warn",
      // Voice stack uses "void promise" / "promise.catch(...)" expression
      // statements as fire-and-forget — the rule's "ignore void" + "allow
      // tagged templates" exceptions cover them.
      "@typescript-eslint/no-unused-expressions": ["error", {
        allowShortCircuit: true,
        allowTernary: true,
        allowTaggedTemplates: true,
      }],
      "no-unused-expressions": "off",
      // Prose-heavy JSX in docs/wizard panels uses literal apostrophes; the
      // rule fires on every contraction otherwise.
      "react/no-unescaped-entities": "off",
      // Newer @eslint/js recommended adds preserve-caught-error; soften so
      // existing voice-stack throws aren't a wall of red. Re-tighten in P6.
      "preserve-caught-error": "off",
    },
  },
  { ignores: ["dist/", "node_modules/", "src-tauri/", "**/*.test.{ts,tsx}"] },
];
