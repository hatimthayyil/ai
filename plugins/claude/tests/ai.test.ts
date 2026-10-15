import type { On, SessionMessage } from 'claude-code'
import { expect, mock, test } from 'claude-code/testing'
import type { Engine } from 'claude-code/testing'

const SESSION = 's1'
const TRANSCRIPT = '/t/s1.jsonl'
const VIEW = '<view>KIWI-9</view>\n'
const NATIVE: SessionMessage[] = [{ role: 'user', text: 'native summary', toolUses: [] }]
const PROJECT = 'a project hook context'
const SESSION_ARGS = ['--harness', 'claude', '--session', SESSION]
const ALL_ARGS = [...SESSION_ARGS, '--transcript', TRANSCRIPT]

type Answer = [exitCode: number, stdout: string, stderr: string]

function ai($: Engine, args: string) {
  return $.command.run({
    command: 'ai',
    args,
    origin: { kind: 'composer' },
    presentation: { isFullscreen: false, columns: 80 },
  })
}

function engine(on: On, answer: (args: readonly string[]) => Answer) {
  const calls: (readonly string[])[] = []
  const compacts: string[] = []
  const logs: string[] = []
  on('session.id', async () => ({ value: SESSION }))
  on('classic.SessionStart', async () => ({ additionalContext: [PROJECT] }))
  on('classic.Stop', async () => ({}))
  on('process.run', async (_$, e) => {
    calls.push(e.argv)
    const [exitCode, stdout, stderr] = answer(e.argv.slice(2))
    return { value: { exitCode, stdout, stderr, isStdoutTruncated: false, isStderrTruncated: false } }
  })
  on('session.compact', async (_$, e) => {
    compacts.push(e.trigger)
    return { messages: NATIVE }
  })
  on('command.run', { command: 'compact' }, async () => {
    compacts.push('/compact')
    return {}
  })
  on('ui.log', async (_$, e) => {
    logs.push(e.text)
    return { value: undefined }
  })
  return { calls, compacts, logs }
}

function start($: Engine, source: 'startup' | 'resume' | 'clear' = 'startup') {
  return $.classic.SessionStart({ source, session_id: SESSION, transcript_path: TRANSCRIPT })
}

test('an attached session compacts into the ai chat view', async ($, on) => {
  const world = engine(on, () => [0, VIEW, ''])
  await start($, 'resume')
  const result = await $.session.compact({ trigger: 'auto', messages: [] })
  expect(result.messages).toEqual([{ role: 'user', text: VIEW, toolUses: [] }])
  expect(world.compacts).toEqual([])
  expect(world.calls).toEqual([['ai', 'chat', 'compact', ...ALL_ARGS]])
})

test('a new session of a thread starts with its view', async ($, on) => {
  const world = engine(on, () => [0, VIEW, ''])
  const result = await start($)
  expect(result.additionalContext).toEqual([PROJECT, VIEW])
  expect(world.calls).toEqual([['ai', 'chat', 'compact', ...ALL_ARGS]])
})

test('a resumed, cleared, unattached or empty session starts as it would', async ($, on) => {
  let answer: Answer = [0, VIEW, '']
  const world = engine(on, () => answer)
  expect((await start($, 'resume')).additionalContext).toEqual([PROJECT])
  expect((await start($, 'clear')).additionalContext).toEqual([PROJECT])
  expect(world.calls).toEqual([])
  answer = [3, '', '']
  expect((await start($)).additionalContext).toEqual([PROJECT])
  answer = [0, '', '']
  expect((await start($)).additionalContext).toEqual([PROJECT])
  expect(world.calls.length).toBe(2)
  expect(world.logs).toEqual([])
})

test('a failed seed starts without the view and says why', async ($, on) => {
  const world = engine(on, () => [1, '', 'no claude on PATH\n'])
  expect((await start($)).additionalContext).toEqual([PROJECT])
  expect(world.logs).toEqual(['ai chat compact failed; starting without the thread. no claude on PATH'])
})

test('an attached session with nothing in its thread compacts natively', async ($, on) => {
  const world = engine(on, () => [0, '', ''])
  const result = await $.session.compact({ trigger: 'manual', messages: [] })
  expect(result.messages).toEqual(NATIVE)
  expect(world.compacts).toEqual(['manual'])
})

test('an unattached session compacts natively', async ($, on) => {
  const world = engine(on, () => [3, '', ''])
  const result = await $.session.compact({ trigger: 'manual', messages: [] })
  expect(result.messages).toEqual(NATIVE)
  expect(world.compacts).toEqual(['manual'])
  expect(world.calls).toEqual([['ai', 'chat', 'compact', ...SESSION_ARGS]])
  expect(world.logs).toEqual([])
})

test('a failed ai chat compact falls back to native and says why', async ($, on) => {
  const world = engine(on, () => [1, '', 'lock held\n'])
  const result = await $.session.compact({ trigger: 'manual', messages: [] })
  expect(result.messages).toEqual(NATIVE)
  expect(world.compacts).toEqual(['manual'])
  expect(world.logs).toEqual(['ai chat compact failed; compacting natively. lock held'])
})

test('a subagent compacts natively without ai chat', async ($, on) => {
  const world = engine(on, () => [0, VIEW, ''])
  await $.session.compact({ trigger: 'auto', agentId: 'a1', messages: [] })
  expect(world.compacts).toEqual(['auto'])
  expect(world.calls).toEqual([])
})

test('/ai attach and detach run ai chat with the session', async ($, on) => {
  const world = engine(on, args => (args[0] === 'attach' ? [0, 'Attached to dev.\n', ''] : [3, '', '']))
  await start($)
  expect((await ai($, 'attach dev')).text).toBe('Attached to dev.')
  expect((await ai($, ' detach ')).text).toBe(
    'This session is not attached to an ai chat thread.',
  )
  expect((await ai($, 'attach')).text).toMatch(/^Usage/)
  expect(world.calls).toEqual([
    ['ai', 'chat', 'compact', ...ALL_ARGS],
    ['ai', 'chat', 'attach', 'dev', ...ALL_ARGS],
    ['ai', 'chat', 'detach', ...ALL_ARGS],
  ])
})

test('/ai compress runs /compact after the command', async ($, on) => {
  const clock = mock.clock(on)
  const world = engine(on, () => [0, '', ''])
  expect((await ai($, 'compress')).text).toBe('Compacting.')
  expect(world.compacts).toEqual([])
  await clock.advance(0)
  expect(world.compacts).toEqual(['/compact'])
})

test('the agent tool queues /compact only when attached', async ($, on) => {
  const clock = mock.clock(on)
  let isAttached = false
  const world = engine(on, () => (isAttached ? [0, '', ''] : [3, '', '']))
  const call = () => $.tool.call({ tool: 'mcp__ai__compress', input: {} })
  expect(await call()).toMatchObject({ result: 'This session is not attached to an ai chat thread.' })
  await clock.advance(0)
  expect(world.compacts).toEqual([])
  isAttached = true
  expect(await call()).toMatchObject({ result: 'Compaction queued for the end of this turn.' })
  await clock.advance(0)
  expect(world.compacts).toEqual(['/compact'])
  expect(world.calls).toEqual([
    ['ai', 'chat', 'sync', ...SESSION_ARGS],
    ['ai', 'chat', 'sync', ...SESSION_ARGS],
  ])
})

test('turn end syncs, and only a real failure is shown', async ($, on) => {
  let answer: Answer = [3, '', '']
  const world = engine(on, () => answer)
  await $.classic.Stop({ session_id: SESSION, transcript_path: TRANSCRIPT, stop_hook_active: false })
  answer = [1, '', 'store locked\n']
  await $.classic.Stop({ session_id: SESSION, transcript_path: TRANSCRIPT, stop_hook_active: false })
  expect(world.calls).toEqual([
    ['ai', 'chat', 'sync', ...ALL_ARGS],
    ['ai', 'chat', 'sync', ...ALL_ARGS],
  ])
  expect(world.logs).toEqual(['ai chat sync failed: store locked'])
})
