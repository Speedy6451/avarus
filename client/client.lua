local ipaddr = "68.46.126.104"
local port = "48228"

local endpoint = "http://" .. ipaddr .. ":" .. port

local idfile = fs.open("id", "r")

local id = nil
local command = nil
local backoff = 0;

if not idfile then
    local fuel = turtle.getFuelLevel()
    local stdin = io.input()
    print("Direction (North, South, East, West):")
    local direction = stdin:read("l")
    print("X:")
    local x = tonumber(stdin:read("l"))
    print("Y:")
    local x = tonumber(stdin:read("l"))
    print("Z:")
    local x = tonumber(stdin:read("l"))
    local y = tonumber(stdin:read("l"))
    local z = tonumber(stdin:read("l"))

    local info = {
        fuel = fuel,
        position = {x, y, z},
        facing = direction,
    }
    -- TODO: get from boot floppy
    local turtleinfo = http.post(
        endpoint .. "/turtle/new",
        textutils.serializeJSON(info),
        { ["Content-Type"] = "application/json" }
    )
    local response = textutils.unserialiseJSON(turtleinfo.readAll())

    idfile = fs.open("id", "w")
    idfile.write(response.id)
    idfile.close()
    os.setComputerLabel(response.name)
    id = response.id
    command = response.command
else
    id = idfile.readAll()
    idfile.close()
end

repeat
    print(command)
    if command == "Wait" then
        sleep(5)
    elseif command == "Forward" then
        turtle.forward()
    elseif command == "Backward" then
        turtle.backward()
    elseif command == "Left" then
        turtle.left()
    elseif command == "Right" then
        turtle.right()
    elseif command == "Update" then
        local req = http.get(endpoint .. "/turtle/client.lua")
        local update = req.readAll()
        req.close()
        local startup = fs.open("startup", "w")
        startup.write(update)
        startup.close()
        os.reboot()
    end

    local ahead = "minecraft:air"
    local above = "minecraft:air"
    local below = "minecraft:air"

    local a,b = turtle.inspect()
    if a then
        ahead = b.name
    end

    local a,b = turtle.inspectUp()
    if a then
        above = b.name
    end

    local a,b = turtle.inspectDown()
    if a then
        below = b.name
    end
    local info = {
        fuel = turtle.getFuelLevel(),
        ahead = ahead,
        above = above,
        below = below
    }

    local rsp = http.post(
        endpoint .. "/turtle/update/" .. id,
        textutils.serializeJSON(info),
        { ["Content-Type"] = "application/json" }
    )
    if rsp then
        backoff = 0
        command = textutils.unserialiseJSON(rsp.readAll())
    else
        print("C&C server offline, waiting " .. backoff .. " seconds")
        sleep(backoff)
        backoff = backoff + 1
    end
until command == "Poweroff"
