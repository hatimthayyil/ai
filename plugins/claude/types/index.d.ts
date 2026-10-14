export type AiTranscript = string

declare module 'claude-code' {
  interface PluginState {
    ai: { transcript: AiTranscript }
  }
}
