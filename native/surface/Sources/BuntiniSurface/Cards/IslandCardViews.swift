// Views for the island's own cards (stream A).
import SwiftUI
import SurfaceKit

struct DropCardView: View {
    let card: Card
    let drop: DropCard
    let send: (Action) -> Void

    var body: some View {
        CardHeading(from: "\(drop.files.count)", title: drop.files.map { ($0 as NSString).lastPathComponent }.joined(separator: ", "))
    }
}

struct GreetingCardView: View {
    let greeting: Greeting

    var body: some View {
        Text(greeting.name.map { "Hi \($0)" } ?? "Hi").font(.callout.weight(.semibold))
    }
}
