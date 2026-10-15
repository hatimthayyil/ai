import type { EngineInterface, Register } from 'claude-code'

const NOT_ATTACHED = 3
const LONG_MS = 600_000
const FLUSH_MS = 1_100
const USAGE = 'Usage: /ai compress | /ai attach <name> | /ai detach'

const transcript = { plugin: 'ai', key: 'transcript' } as const

type Ran = { exitCode: number; stdout: string; stderr: string }

async function chat($: EngineInterface, args: string[], timeoutMs?: number): Promise<Ran> {
  try {
    return await $.process.run(['ai', 'chat', ...args], { timeoutMs })
  } catch (err) {
    return { exitCode: -1, stdout: '', stderr: String(err) }
  }
}

function sessionArgs(id: string, path: string | undefined): string[] {
  return ['--harness', 'claude', '--session', id, ...(path === undefined ? [] : ['--transcript', path])]
}

async function session($: EngineInterface): Promise<string[]> {
  return sessionArgs(await $.session.id(), (await $.state.get(transcript)).value)
}

async function view($: EngineInterface, args: string[], fallback: string): Promise<string | undefined> {
  const ran = await chat($, ['compact', ...args], LONG_MS)
  if (ran.exitCode === 0 && ran.stdout !== '') return ran.stdout
  if (ran.exitCode !== 0 && ran.exitCode !== NOT_ATTACHED) {
    $.ui.log(`ai chat compact failed; ${fallback}. ${ran.stderr.trim()}`)
  }
  return undefined
}

async function sync($: EngineInterface, args: string[]): Promise<void> {
  const ran = await chat($, ['sync', ...args])
  if (ran.exitCode !== 0 && ran.exitCode !== NOT_ATTACHED) $.ui.log(`ai chat sync failed: ${ran.stderr.trim()}`)
}

function reply(ran: Ran): string {
  if (ran.exitCode === 0) return ran.stdout.trim()
  if (ran.exitCode === NOT_ATTACHED) return 'This session is not attached to an ai chat thread.'
  return `ai chat failed: ${ran.stderr.trim()}`
}

function queueCompact($: EngineInterface): void {
  $.clock.after(0, () => $.command.run({ command: 'compact' }))
}

export const register: Register = on => {
  on('session.start', async ($, e, next) => {
    await $.command.register({
      name: 'ai',
      description: 'ai chat: compress, attach <name>, detach',
      argumentHint: 'compress | attach <name> | detach',
    })
    await $.tool.register({
      name: 'compress',
      description:
        'Compacts this conversation into its ai chat thread view once the current turn ends. Only for a session attached to an ai chat thread.',
      inputSchema: { type: 'object', properties: {} },
    })
    return next(e)
  })

  on('classic.SessionStart', async ($, e, next) => {
    await $.state.set(transcript, e.transcript_path)
    const started = await next(e)
    if (e.source !== 'startup') return started
    const seed = await view($, sessionArgs(e.session_id, e.transcript_path), 'starting without the thread')
    if (seed === undefined) return started
    return { ...started, additionalContext: [...(started.additionalContext ?? []), seed] }
  })

  on('session.compact', async ($, e, next) => {
    if (e.agentId !== undefined) return next(e)
    await $.clock.sleep(FLUSH_MS)
    const text = await view($, await session($), 'compacting natively')
    if (text === undefined) return next(e)
    return { messages: [{ role: 'user', text, toolUses: [] }] }
  })

  on('classic.Stop', async ($, e, next) => {
    const args = sessionArgs(e.session_id, e.transcript_path)
    $.clock.after(FLUSH_MS, () => sync($, args))
    return next(e)
  })

  on('session.end', async ($, e, next) => {
    const ended = await next(e)
    await sync($, sessionArgs(e.sessionId, (await $.state.get(transcript)).value))
    return ended
  })

  on('command.run', { command: 'ai' }, async ($, e) => {
    const [sub, ...rest] = e.args.trim().split(/\s+/)
    if (sub === 'compress' && rest.length === 0) {
      queueCompact($)
      return { text: 'Compacting.' }
    }
    if (sub === 'attach' && rest.length === 1) {
      return { text: reply(await chat($, ['attach', rest[0]!, ...(await session($))], LONG_MS)) }
    }
    if (sub === 'detach' && rest.length === 0) {
      return { text: reply(await chat($, ['detach', ...(await session($))])) }
    }
    return { text: USAGE }
  })

  on('tool.call', { tool: 'mcp__ai__compress' }, async $ => {
    const ran = await chat($, ['sync', ...(await session($))])
    if (ran.exitCode !== 0) return { result: reply(ran) }
    queueCompact($)
    return { result: 'Compaction queued for the end of this turn.' }
  })
}
