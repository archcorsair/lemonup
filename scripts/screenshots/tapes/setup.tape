Source tapes/common.tape
Output shots/setup.txt

# ./lemonup-fresh runs with an empty HOME (fresh/ in the sandbox), so setup
# opens. Its search finds the fake install in ~/Games/World of Warcraft and
# lists Retail and WoW Forever with their addon counts (step 1, found).
Hide
Type "clear && ./lemonup-fresh"
Enter
Sleep 5s
Show
Sleep 1500ms
Screenshot shots/setup.png

# Ctrl+C quits from anywhere; nothing is saved.
Hide
Ctrl+C
Sleep 1s
