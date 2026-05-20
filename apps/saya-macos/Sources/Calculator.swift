import Foundation

/// Tiny arithmetic evaluator for the launcher panel.
///
/// Supports: integers and decimals, unary minus, `+ - * /`, parentheses,
/// and the percent operator (`50%` → 0.5). Rejects anything else.
///
/// We hand-roll a recursive-descent parser instead of using `NSExpression`
/// because `NSExpression.init(format:)` throws `NSInvalidArgumentException`
/// on malformed input, and that exception isn't catchable in pure Swift.
enum Calculator {
    static func evaluate(_ raw: String) -> Double? {
        let normalized = raw
            .replacingOccurrences(of: "×", with: "*")
            .replacingOccurrences(of: "÷", with: "/")
            .replacingOccurrences(of: "−", with: "-")
        let trimmed = normalized.trimmingCharacters(in: .whitespaces)
        guard !trimmed.isEmpty else { return nil }

        var parser = Parser(chars: Array(trimmed))
        guard let value = parser.parseExpr() else { return nil }
        parser.skipSpaces()
        guard parser.i == parser.chars.count else { return nil }
        // Require at least one operator OR parentheses — bare numbers ("42")
        // shouldn't shadow the launcher results.
        guard parser.sawOperator else { return nil }
        guard value.isFinite else { return nil }
        return value
    }

    /// Format with thousands separator and up to 10 significant decimals.
    static func format(_ value: Double) -> String {
        let formatter = NumberFormatter()
        formatter.usesGroupingSeparator = true
        formatter.groupingSeparator = ","
        formatter.minimumFractionDigits = 0
        formatter.maximumFractionDigits = 10
        return formatter.string(from: NSNumber(value: value)) ?? "\(value)"
    }
}

private struct Parser {
    let chars: [Character]
    var i: Int = 0
    var sawOperator: Bool = false

    mutating func skipSpaces() {
        while i < chars.count && chars[i].isWhitespace { i += 1 }
    }

    mutating func peek() -> Character? {
        skipSpaces()
        return i < chars.count ? chars[i] : nil
    }

    mutating func consume(_ c: Character) -> Bool {
        skipSpaces()
        if i < chars.count && chars[i] == c {
            i += 1
            return true
        }
        return false
    }

    // expr   = term (('+'|'-') term)*
    mutating func parseExpr() -> Double? {
        guard var left = parseTerm() else { return nil }
        while let op = peek(), op == "+" || op == "-" {
            i += 1
            sawOperator = true
            guard let right = parseTerm() else { return nil }
            left = (op == "+") ? left + right : left - right
        }
        return left
    }

    // term   = factor (('*'|'/') factor)*
    mutating func parseTerm() -> Double? {
        guard var left = parseFactor() else { return nil }
        while let op = peek(), op == "*" || op == "/" {
            i += 1
            sawOperator = true
            guard let right = parseFactor() else { return nil }
            if op == "/" {
                if right == 0 { return nil }
                left /= right
            } else {
                left *= right
            }
        }
        // Postfix percent: 50% → 0.5
        if consume("%") {
            sawOperator = true
            left /= 100
        }
        return left
    }

    // factor = '-' factor | '(' expr ')' | number
    mutating func parseFactor() -> Double? {
        skipSpaces()
        if consume("-") {
            guard let v = parseFactor() else { return nil }
            return -v
        }
        if consume("(") {
            sawOperator = true
            guard let v = parseExpr() else { return nil }
            guard consume(")") else { return nil }
            return v
        }
        return parseNumber()
    }

    mutating func parseNumber() -> Double? {
        skipSpaces()
        let start = i
        var sawDigit = false
        while i < chars.count && chars[i].isNumber {
            i += 1
            sawDigit = true
        }
        if i < chars.count && chars[i] == "." {
            i += 1
            while i < chars.count && chars[i].isNumber {
                i += 1
                sawDigit = true
            }
        }
        guard sawDigit else { return nil }
        return Double(String(chars[start..<i]))
    }
}
