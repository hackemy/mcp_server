using golang i need a real MCP server that use HTTP POST streaming. Use github.com/gofiber/fiber/v3 and one route /mcp which accepts POSTS only.  
  You goal is to make it compliant with the real MCP specification made by anthropic. The project must have a tools.json which describes which      
  tools to list and which can be called and a similar one for resources.json. tools/list resources/list tools/call and resources/read are staples.  
  Make as many unit tests you can to verify the functionality. Isolate the main handler because this might run in AWS as a lambda, so fiber is more 
   for local development, OR if i wanna run it behind nginx. nginx would do https termination, so do not worry about that, IF there is a better way 
   for nginx to work with golang skip fiber and make me a nginx config file and we can test. Make some speculative tools.json for now with          
  implementations in a subfolder called tools/* which the handlers of each tool will live. I guess we can have the tools/tools.json and             
  resources/resources.json nested in the appropriate location.