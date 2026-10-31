// Apple's on-device language model (Foundation Models), called from Rust over a C interface.
// Every function takes and returns JSON as a C string, or returns NULL when the model is
// unavailable or fails. The model runs on the Mac; nothing here touches the network.

import Foundation
import FoundationModels

/// A JSON object schema built at run time, so the shim builds without Xcode's macro plugins.
private func object(_ name: String, _ properties: [(String, String, DynamicGenerationSchema)]) -> GenerationSchema? {
    let root = DynamicGenerationSchema(
        name: name,
        properties: properties.map { DynamicGenerationSchema.Property(name: $0.0, description: $0.1, schema: $0.2) })
    return try? GenerationSchema(root: root, dependencies: [])
}

private let text = DynamicGenerationSchema(type: String.self)
private let number = DynamicGenerationSchema(type: Int.self)

private let planSchema = object("Plan", [
    ("terms", "Names, topics, ids and distinctive words to search for, lowercase, without question words or dates",
     DynamicGenerationSchema(arrayOf: text, minimumElements: 1, maximumElements: 6)),
    ("time", "When it happened, if the question says so",
     DynamicGenerationSchema(name: "Time", anyOf: ["any time", "today", "yesterday", "this week", "last week", "this month"])),
])

private let answerSchema = object("Answer", [
    ("text", "One to three short sentences that answer the question from the notes only, quoting the exact words that matter. If the notes do not answer it, say that nothing was found.", text),
    ("note", "The number of the note that supports the answer, or 0 if none does", number),
])

private let judgeSchema = object("Judgment", [
    ("share", "How much of this step a software agent with access to these apps could do without the user",
     DynamicGenerationSchema(name: "Share", anyOf: ["not at all", "a little", "half", "most", "nearly all"])),
    ("reason", "One short sentence on what the agent could do or what still needs the user", text),
])

struct Step: Codable {
    let name: String
    let category: String
    let apps: [String]
    let objects: [String]
}

struct Note: Codable {
    let app: String
    let title: String
    let ts: Int64
    let lines: [String]
}

struct AnswerInput: Codable {
    let question: String
    let notes: [Note]
}

private func blocking<T>(_ work: @escaping () async throws -> T) -> T? {
    let done = DispatchSemaphore(value: 0)
    var result: T?
    Task.detached {
        result = try? await work()
        done.signal()
    }
    done.wait()
    return result
}

private func respond(_ session: LanguageModelSession, _ prompt: String, _ schema: GenerationSchema?) -> UnsafeMutablePointer<CChar>? {
    guard let schema else { return nil }
    let json = blocking {
        try await session.respond(to: prompt, schema: schema).content.jsonString
    }
    return json.map { strdup($0) }
}

private func planInstructions() -> String {
    """
    You turn a question about what the user saw, wrote or heard on their Mac into a search. \
    Today is \(day.string(from: Date())).
    """
}

/// A plan session whose model and instructions are already loaded, taken by the next plan call.
private var warmPlan: LanguageModelSession?
private let warmLock = NSLock()

/// Loads the model while the user types, so the first answer skips the cold start.
@_cdecl("sextant_language_prewarm")
public func sextantLanguagePrewarm() {
    guard case .available = SystemLanguageModel.default.availability else { return }
    let session = LanguageModelSession(instructions: planInstructions())
    session.prewarm()
    warmLock.withLock { warmPlan = session }
}

private let day: DateFormatter = {
    let formatter = DateFormatter()
    formatter.dateFormat = "EEEE yyyy-MM-dd"
    formatter.locale = Locale(identifier: "en_US_POSIX")
    return formatter
}()

private let stamp: DateFormatter = {
    let formatter = DateFormatter()
    formatter.dateFormat = "EEE yyyy-MM-dd HH:mm"
    formatter.locale = Locale(identifier: "en_US_POSIX")
    return formatter
}()

/// Input: the question. Output: `{"terms": [...], "time": "yesterday"}`.
@_cdecl("sextant_language_plan")
public func sextantLanguagePlan(_ question: UnsafePointer<CChar>) -> UnsafeMutablePointer<CChar>? {
    let question = String(cString: question)
    let warm = warmLock.withLock {
        defer { warmPlan = nil }
        return warmPlan
    }
    return respond(warm ?? LanguageModelSession(instructions: planInstructions()), question, planSchema)
}

/// Called with the answer text so far, each time the model adds to it.
public typealias TextSink = @convention(c) (UnsafeMutableRawPointer?, UnsafePointer<CChar>) -> Void

/// Input: `{"question": "...", "notes": [{"app", "title", "ts", "lines"}]}`.
/// Output: `{"text": "...", "note": n}`, where `note` counts from 1. Streams the text to `sink`.
@_cdecl("sextant_language_answer")
public func sextantLanguageAnswer(
    _ input: UnsafePointer<CChar>, _ sink: TextSink, _ context: UnsafeMutableRawPointer?
) -> UnsafeMutablePointer<CChar>? {
    guard let input = try? JSONDecoder().decode(AnswerInput.self, from: Data(String(cString: input).utf8)) else {
        return nil
    }
    let notes = input.notes.enumerated().map { index, note in
        let when = stamp.string(from: Date(timeIntervalSince1970: TimeInterval(note.ts)))
        let lines = note.lines.map { "  \($0)" }.joined(separator: "\n")
        return "[\(index + 1)] \(note.app), \"\(note.title)\", \(when)\n\(lines)"
    }.joined(separator: "\n\n")
    let today = day.string(from: Date())
    let session = LanguageModelSession(instructions: """
        You help the user remember what they saw, wrote or heard on their Mac. Today is \(today). \
        Answer only from the numbered notes, which are text read from their screen and calls. \
        Never guess or add facts.
        """)
    guard let schema = answerSchema else { return nil }
    let prompt = "Question: \(input.question)\n\nNotes:\n\(notes)"
    let json = blocking {
        var last: GeneratedContent?
        for try await snapshot in session.streamResponse(to: prompt, schema: schema) {
            last = snapshot.content
            if let text = try? snapshot.content.value(String.self, forProperty: "text") {
                text.withCString { sink(context, $0) }
            }
        }
        return last?.jsonString
    }
    return json.flatMap { $0 }.map { strdup($0) }
}

/// Input: `{"name", "category", "apps", "objects"}` for one recurring step.
/// Output: `{"share": "most", "reason": "..."}`.
@_cdecl("sextant_language_judge")
public func sextantLanguageJudge(_ input: UnsafePointer<CChar>) -> UnsafeMutablePointer<CChar>? {
    guard let step = try? JSONDecoder().decode(Step.self, from: Data(String(cString: input).utf8)) else {
        return nil
    }
    let objects = step.objects.map { "- \($0)" }.joined(separator: "\n")
    return respond(LanguageModelSession(instructions: """
        You judge how much of a recurring computer task a software agent could take over. The agent \
        can read and write in the user's apps but cannot make judgment calls the user would want to \
        make, like approving money, choosing what to say to a person or creative decisions. Judge \
        from the step alone and be sober: most steps need the user for something.
        """), """
        Step: \(step.name)
        Kind: \(step.category)
        Apps: \(step.apps.joined(separator: ", "))
        What it works on, with numbers and names replaced:
        \(objects)
        """, judgeSchema)
}

@_cdecl("sextant_language_free")
public func sextantLanguageFree(_ pointer: UnsafeMutablePointer<CChar>?) {
    free(pointer)
}
